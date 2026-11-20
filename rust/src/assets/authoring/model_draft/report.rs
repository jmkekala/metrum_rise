// SPDX-License-Identifier: GPL-3.0-or-later

//! The stale-geometry report (docs/asset_editor.md "Draft command"): what a rebuilt model
//! changes for an asset that already exists, and information lines that do not mark it stale.
//! Read-only; every line names the editor field it concerns. O(published files + doors + site
//! items × (footprint + item) log(footprint + item)), once per draft.

use super::rules::LOT_FIT_TOLERANCE_M;
use super::sidecar::{Facts, Role};
use crate::assets::asset::geometry::{fits_lot, intersection_area, signed_area};
use crate::assets::asset::validation::{ZONE_CELL_M, site_anchor_footprint};
use crate::assets::{AnchorType, AssetManifest, MeshPart};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The entrance anchor may stand this far from the main door in plan before it is reported
/// (Q6, decided 2026-11-20); the acceptance check uses the same bound.
pub(crate) const ENTRANCE_TOLERANCE_M: f32 = 0.5;
/// The entrance anchor may turn this far from the door's facing before it is reported (Q6).
pub(crate) const ENTRANCE_TOLERANCE_DEG: f32 = 15.0;
/// With a ledger entry, a door that moved this far is reported on its own.
const DOOR_MOVE_M: f64 = 0.05;

/// A site item reported only when it shares more than this with the footprint: a surface whose
/// edge lies on a wall touches it and shares nothing, and float noise along such an edge stays
/// far below this. A surface holding all but this much of the footprint is a pad under it.
pub(crate) const OVERLAP_TOLERANCE_M2: f64 = 0.01;

/// One reported difference.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Line {
    /// Which of the report's checks found it: `files`, `facts`, `lot_fit`, `entrance`,
    /// `schemes`, `capacity`, `site_layout` or `role`.
    pub(crate) check: &'static str,
    /// The editor field the owner updates.
    pub(crate) field: &'static str,
    pub(crate) message: String,
    /// A stale line marks the asset stale (exit status 2); an information line does not.
    #[serde(skip)]
    pub(crate) stale: bool,
}

pub(crate) fn line(check: &'static str, field: &'static str, message: String) -> Line {
    Line {
        check,
        field,
        message,
        stale: true,
    }
}

fn info(check: &'static str, field: &'static str, message: String) -> Line {
    Line {
        stale: false,
        ..line(check, field, message)
    }
}

/// Every report line for `facts` against the installed `manifest`. `published` maps each file
/// the asset publishes (relative to its folder) to its SHA-256; `ledger` is the sidecar the
/// asset was drafted from, when it has a ledger entry.
pub(crate) fn stale_report(
    facts: &Facts,
    ledger: Option<&Facts>,
    manifest: &AssetManifest,
    published: &BTreeMap<String, String>,
) -> Vec<Line> {
    let mut lines = files(facts, published);
    if let Some(ledger) = ledger {
        lines.extend(changed_facts(facts, ledger));
    }
    let part = model_part(facts, manifest);
    if let (Some(part), Some(building)) = (part, &manifest.building) {
        let plan = part.plan_bounds(bounds32(facts));
        let half = [building.lot_width_cells, building.lot_depth_cells]
            .map(|cells| f32::from(cells) * ZONE_CELL_M * 0.5);
        if !fits_lot(plan, half, LOT_FIT_TOLERANCE_M as f32) {
            lines.push(line(
                "lot_fit",
                "mesh_parts",
                format!(
                    "the new LOD0 bounds through part '{}' span x {:.2}..{:.2}, z {:.2}..{:.2}, outside the lot ±{}×±{} m",
                    part.name, plan[0][0], plan[1][0], plan[0][1], plan[1][1], half[0], half[1]
                ),
            ));
        }
        lines.extend(entrance(facts, ledger, manifest, part));
        lines.extend(site_layout(facts, ledger, manifest, part));
    } else if part.is_none() {
        lines.push(line(
            "lot_fit",
            "mesh_parts",
            "the asset has no mesh part to place the new model through".into(),
        ));
    }
    lines.extend(schemes(facts, manifest));
    let households = manifest
        .building
        .as_ref()
        .and_then(|building| building.household_capacity)
        .unwrap_or(0);
    if households != facts.dwellings {
        lines.push(line(
            "capacity",
            "household_capacity",
            format!(
                "household_capacity is {households}, the model has {} dwellings",
                facts.dwellings
            ),
        ));
    }
    lines
}

/// The part that shows the model: the one whose `LOD0` file is the sidecar's, else the first.
fn model_part<'a>(facts: &Facts, manifest: &'a AssetManifest) -> Option<&'a MeshPart> {
    manifest
        .mesh_parts
        .iter()
        .find(|part| {
            part.lods
                .first()
                .is_some_and(|lod| lod.file == facts.lods[0].file)
        })
        .or(manifest.mesh_parts.first())
}

pub(crate) fn bounds32(facts: &Facts) -> [[f32; 3]; 2] {
    [facts.bounds.min, facts.bounds.max].map(|corner| corner.map(|v| v as f32))
}

// Published tiers and textures against the sidecar's files of the same name. A name may be
// published twice (the default albedo, beside the `.glb` and under `colours/`).
fn files(facts: &Facts, published: &BTreeMap<String, String>) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut sidecar: BTreeMap<&str, (&str, &'static str)> = BTreeMap::new();
    for lod in &facts.lods {
        sidecar.insert(&lod.file, (&lod.sha256, "mesh_parts"));
    }
    let textures = [
        Some(&facts.textures.orm),
        Some(&facts.textures.normal),
        facts.textures.emission.as_ref(),
    ];
    for texture in textures.into_iter().flatten() {
        sidecar.insert(&texture.file, (&texture.sha256, "appearance"));
    }
    for variant in &facts.variants {
        sidecar.insert(&variant.albedo, (&variant.sha256, "appearance"));
    }
    let mut seen = BTreeSet::new();
    for (path, digest) in published {
        let name = path.rsplit('/').next().unwrap_or(path);
        match sidecar.get(name) {
            Some((expected, field)) => {
                seen.insert(name);
                if digest != expected {
                    lines.push(line(
                        "files",
                        field,
                        format!("{path} differs from the model's {name}"),
                    ));
                }
            }
            None => lines.push(line(
                "files",
                if path.starts_with("colours/") {
                    "appearance"
                } else {
                    "mesh_parts"
                },
                format!("{path} is published but the model has no file of that name"),
            )),
        }
    }
    for (name, (_, field)) in &sidecar {
        if !seen.contains(name) {
            lines.push(line(
                "files",
                field,
                format!("the model's {name} is not published in the asset"),
            ));
        }
    }
    lines
}

/// Whether two footprints are the same outline: equal point lists up to the start point. The
/// writer always winds positively but may start anywhere, so a rotation of the list is no
/// change; points compare exactly. O(n²) for n points, once per draft.
pub(crate) fn same_outline(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    a.len() == b.len()
        && (a.is_empty()
            || (0..b.len()).any(|start| (0..a.len()).all(|i| a[i] == b[(start + i) % b.len()])))
}

// Sidecar fields that differ from the ledger copy the asset was drafted from.
fn changed_facts(facts: &Facts, ledger: &Facts) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut compare = |changed: bool, name: &str, field: &'static str| {
        if changed {
            lines.push(line(
                "facts",
                field,
                format!("{name} changed since the asset was drafted"),
            ));
        }
    };
    compare(facts.bounds != ledger.bounds, "bounds", "mesh_parts");
    compare(
        !same_outline(&facts.footprint, &ledger.footprint),
        "footprint",
        "site_surfaces",
    );
    compare(facts.storeys != ledger.storeys, "storeys", "flat_size_m2");
    compare(
        facts.dwellings != ledger.dwellings,
        "dwellings",
        "household_capacity",
    );
    compare(facts.variants != ledger.variants, "variants", "appearance");
    let old: BTreeMap<_, _> = ledger.doors.iter().map(|d| (d.id.as_str(), d)).collect();
    let new: BTreeMap<_, _> = facts.doors.iter().map(|d| (d.id.as_str(), d)).collect();
    for id in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        if old.get(id) != new.get(id) {
            compare(true, &format!("door '{id}'"), "anchors");
        }
    }
    lines
}

fn entrance(
    facts: &Facts,
    ledger: Option<&Facts>,
    manifest: &AssetManifest,
    part: &MeshPart,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let door = facts.main_door();
    if let Some(old) = ledger.map(Facts::main_door) {
        if old.id != door.id {
            lines.push(line(
                "entrance",
                "anchors",
                format!("the main door is now '{}', was '{}'", door.id, old.id),
            ));
        } else {
            let moved = (0..3)
                .map(|axis| (door.position[axis] - old.position[axis]).powi(2))
                .sum::<f64>()
                .sqrt();
            if moved >= DOOR_MOVE_M {
                lines.push(line(
                    "entrance",
                    "anchors",
                    format!("the main door '{}' moved {moved:.2} m", door.id),
                ));
            }
        }
    }
    let Some(anchor) = manifest
        .anchors
        .iter()
        .find(|a| a.anchor_type == AnchorType::Entrance && a.name == "main")
    else {
        lines.push(line(
            "entrance",
            "anchors",
            "the asset has no entrance named main".into(),
        ));
        return lines;
    };
    let (distance, angle) = door_offset(part, door, anchor.position, anchor.forward);
    if distance > ENTRANCE_TOLERANCE_M || angle > ENTRANCE_TOLERANCE_DEG {
        lines.push(line(
            "entrance",
            "anchors",
            format!(
                "entrance/main stands {distance:.2} m from the main door '{}' in plan and turns {angle:.1}° from its facing",
                door.id
            ),
        ));
    }
    lines
}

/// The plan distance from an anchor to `door` through `part`, and the angle in degrees between
/// the anchor's forward and the door's facing, both in the lot frame.
pub(crate) fn door_offset(
    part: &MeshPart,
    door: &super::sidecar::Door,
    position: [f32; 3],
    forward: [f32; 3],
) -> (f32, f32) {
    let transform = part.local_transform();
    let point = transform.transform_point3(glam::Vec3::from_array(door.position.map(|v| v as f32)));
    let facing = transform.transform_vector3(glam::Vec3::from_array(door.facing.map(|v| v as f32)));
    let distance = glam::Vec2::new(point.x - position[0], point.z - position[2]).length();
    let facing = glam::Vec2::new(facing.x, facing.z).normalize_or_zero();
    let forward = glam::Vec2::new(forward[0], forward[2]).normalize_or_zero();
    let angle = facing.dot(forward).clamp(-1.0, 1.0).acos().to_degrees();
    (distance, angle)
}

fn schemes(facts: &Facts, manifest: &AssetManifest) -> Vec<Line> {
    let schemes: BTreeSet<&str> = manifest
        .building
        .as_ref()
        .and_then(|building| building.appearance.as_ref())
        .into_iter()
        .flat_map(|appearance| appearance.schemes.iter().map(|s| s.id.as_str()))
        .collect();
    let variants: BTreeSet<&str> = facts.variants.iter().map(|v| v.id.as_str()).collect();
    let mut lines: Vec<Line> = variants
        .difference(&schemes)
        .map(|id| {
            line(
                "schemes",
                "appearance",
                format!("variant '{id}' has no colour scheme"),
            )
        })
        .collect();
    lines.extend(schemes.difference(&variants).map(|id| {
        line(
            "schemes",
            "appearance",
            format!("colour scheme '{id}' has no variant in the model"),
        )
    }));
    lines
}

// Site items the footprint, placed through the present part, stands on. Touching is no overlap;
// a site surface holding the whole footprint is a pad under the building and is not reported.
// Any other overlap is information, since the owner may have laid it so, unless the ledger shows
// the footprint changed since the draft: then the model moved onto the item and it is stale.
fn site_layout(
    facts: &Facts,
    ledger: Option<&Facts>,
    manifest: &AssetManifest,
    part: &MeshPart,
) -> Vec<Line> {
    let transform = part.local_transform();
    let footprint: Vec<[f32; 2]> = facts
        .footprint
        .iter()
        .map(|&[x, z]| {
            let point = transform.transform_point3(glam::Vec3::new(x as f32, 0.0, z as f32));
            [point.x, point.z]
        })
        .collect();
    let footprint_m2 = f64::from(signed_area(&footprint).abs());
    let changed = ledger.is_some_and(|ledger| !same_outline(&ledger.footprint, &facts.footprint));
    let mut lines = Vec::new();
    let mut overlap = |item: String, field: &'static str, shared_m2: f64| {
        if shared_m2 <= OVERLAP_TOLERANCE_M2 {
            return;
        }
        let message = format!(
            "{item} lies under {shared_m2:.2} m² of the building's {footprint_m2:.2} m² footprint"
        );
        lines.push(if changed {
            line(
                "site_layout",
                field,
                format!("{message}; the footprint changed since the asset was drafted"),
            )
        } else {
            info("site_layout", field, message)
        });
    };
    for surface in &manifest.site_surfaces {
        let shared = intersection_area(&footprint, &surface.vertices);
        if footprint_m2 - shared > OVERLAP_TOLERANCE_M2 {
            overlap(
                format!("site surface '{}'", surface.name),
                "site_surfaces",
                shared,
            );
        }
    }
    let plantings = manifest
        .building
        .iter()
        .flat_map(|building| &building.yard_planting);
    for area in plantings {
        overlap(
            format!("planting area '{}'", area.name),
            "site_surfaces",
            intersection_area(&footprint, &area.vertices),
        );
    }
    for anchor in &manifest.anchors {
        if let Some(corners) = site_anchor_footprint(anchor) {
            overlap(
                format!("{:?} anchor '{}'", anchor.anchor_type, anchor.name),
                "anchors",
                intersection_area(&footprint, &corners),
            );
        }
    }
    lines
}

/// Information lines for each role input that differs from the existing asset's manifest, so a
/// mistyped asset id that hits a real asset is noticed. `lot_cells` is compared only when the
/// role gives it. None marks the asset stale.
pub(crate) fn role_mismatches(role: &Role, manifest: &AssetManifest) -> Vec<Line> {
    let Some(building) = &manifest.building else {
        return vec![info(
            "role",
            "asset_id",
            "the existing asset is no building".into(),
        )];
    };
    let quoted = |value: Option<&str>| value.map_or("none".to_owned(), |v| format!("'{v}'"));
    let zone = building
        .zone_type
        .map(|zone| format!("{zone:?}").to_ascii_lowercase());
    let mut fields: Vec<(&'static str, String, String)> = vec![
        ("level", role.level.to_string(), building.level.to_string()),
        (
            "zone_type",
            quoted(Some(&role.zone_type)),
            quoted(zone.as_deref()),
        ),
        (
            "density",
            quoted(Some(&role.density)),
            quoted(building.density.as_deref()),
        ),
    ];
    if let Some(lot) = role.lot_cells {
        fields.push((
            "lot_cells",
            format!("{} × {}", lot[0], lot[1]),
            format!(
                "{} × {}",
                building.lot_width_cells, building.lot_depth_cells
            ),
        ));
    }
    fields.push((
        "asset_set",
        quoted(role.asset_set.as_deref()),
        quoted(manifest.asset_set.as_deref()),
    ));
    fields.push((
        "economy_profile",
        quoted(role.economy_profile.as_deref()),
        quoted(building.economy_profile.as_deref()),
    ));
    fields
        .into_iter()
        .filter(|(_, wanted, present)| wanted != present)
        .map(|(field, wanted, present)| {
            info(
                "role",
                field,
                format!("the role gives {field} {wanted}, the existing asset has {present}"),
            )
        })
        .collect()
}
