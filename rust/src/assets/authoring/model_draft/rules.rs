// SPDX-License-Identifier: GPL-3.0-or-later

//! The derivation rules R1-R9 (docs/asset_editor.md "Derivation rules", `TOOLS-33`): pure,
//! deterministic functions of the sidecar, the tiers' material names and the role inputs.
//! None reads the clock or an existing asset. All are O(tiers + variants + doors).

use super::sidecar::{Door, Facts, Role};
use crate::assets::asset::validation::ZONE_CELL_M;
use serde_json::{Value, json};

/// The editor's lot fit tolerance (`_mesh_part_fits_in_lot`), in metres.
pub(crate) const LOT_FIT_TOLERANCE_M: f64 = 0.01;
/// R6: each tier's near distance; a fifth tier is an error.
pub(crate) const LOD_BANDS_M: [f64; 4] = [0.0, 35.0, 70.0, 120.0];
/// R8: the editor's thumbnail capture size and WebP quality (`thumbnail_capture.gd`).
pub(crate) const THUMBNAIL_SIZE: [i32; 2] = [1024, 768];
pub(crate) const THUMBNAIL_QUALITY: f32 = 0.9;
pub(crate) const THUMBNAIL_FILE: &str = "thumbnail.webp";
/// R9: the window brightness every hand asset carries.
const WINDOW_BRIGHTNESS: f64 = 3.0;
/// R5: the flat size cap of a building with one dwelling (Q4, decided 2026-11-20).
const SINGLE_DWELLING_FLAT_CAP_M2: f64 = 80.0;
/// R5: share of the wall-plate area a household counts as its flat.
const FLAT_SHARE_OF_PLATE: f64 = 0.8;

/// R1, a detached house's yard: the free space the default lot of a one-dwelling building
/// leaves on each side across and in front of the part at its R2 position. The owner's starting
/// values (2026-11-20), judged against four houses of similar size.
const HOUSE_SIDE_YARD_M: f64 = 3.0;
const HOUSE_FRONT_YARD_M: f64 = 4.0;

/// Lot cells covering one extent, with the editor's fit tolerance.
pub(crate) fn lot_cells(extent_m: f64) -> u16 {
    ((extent_m - LOT_FIT_TOLERANCE_M) / f64::from(ZONE_CELL_M))
        .ceil()
        .max(1.0) as u16
}

/// The tightest lot `[width, depth]` the `LOD0` bounds fit, centred across and back-flush.
pub(crate) fn tight_lot(facts: &Facts) -> [u16; 2] {
    let [min, max] = [facts.bounds.min, facts.bounds.max];
    [lot_cells(max[0] - min[0]), lot_cells(max[2] - min[2])]
}

/// R1: the role's `lot_cells` when it gives them, refused when the model does not fit them at
/// its R2 position; else for one dwelling (a detached house) the smallest lot with its yard,
/// and for anything else the tightest lot. Apartment blocks and shops vary too much to derive:
/// their role orders the lot.
pub(crate) fn lot(facts: &Facts, ordered: Option<[u16; 2]>) -> Result<[u16; 2], String> {
    let tight = tight_lot(facts);
    if let Some(lot) = ordered {
        if lot[0] < tight[0] || lot[1] < tight[1] {
            return Err(format!(
                "role lot_cells {lot:?} is too small: the model needs at least {tight:?} cells (width, depth)"
            ));
        }
        return Ok(lot);
    }
    if facts.dwellings != 1 {
        return Ok(tight);
    }
    let [min, max] = [facts.bounds.min, facts.bounds.max];
    Ok([
        lot_cells(max[0] - min[0] + 2.0 * HOUSE_SIDE_YARD_M),
        lot_cells(max[2] - min[2] + HOUSE_FRONT_YARD_M),
    ])
}

/// R2: centred across, bounds on the back lot edge. A starting position only (Q3).
pub(crate) fn part_position(facts: &Facts, lot_depth_cells: u16) -> [f64; 3] {
    let [min, max] = [facts.bounds.min, facts.bounds.max];
    let half_depth = f64::from(lot_depth_cells) * f64::from(ZONE_CELL_M) * 0.5;
    // `+ 0.0` turns a centred model's −0 into 0, so the manifest never reads `-0`.
    [-(min[0] + max[0]) * 0.5 + 0.0, 0.0, -half_depth - min[2]]
}

/// R4: the main door on its wall plane at ground level, facing out of its wall.
pub(crate) fn entrance(part: [f64; 3], door: &Door) -> Value {
    json!({
        "anchor_type": "entrance",
        "name": "main",
        "position": [part[0] + door.position[0], 0.0, part[2] + door.position[2]],
        "forward": door.facing,
    })
}

/// R5 for housing: 0.8 × the wall-plate area per dwelling, to the nearest 10 m² (halves
/// away from zero), at most 80 m² for one dwelling. `None` for no dwelling or a flat that
/// rounds to nothing.
pub(crate) fn flat_size_m2(facts: &Facts) -> Option<f64> {
    if facts.dwellings == 0 {
        return None;
    }
    let area: f64 = facts.storeys.iter().map(|storey| storey.area_m2).sum();
    let mut flat = (FLAT_SHARE_OF_PLATE * area / f64::from(facts.dwellings) / 10.0).round() * 10.0;
    if facts.dwellings == 1 {
        flat = flat.min(SINGLE_DWELLING_FLAT_CAP_M2);
    }
    (flat > 0.0).then_some(flat)
}

/// R6: the editor's bands, `[min, max]` per tier; the last tier has no maximum.
pub(crate) fn lod_bands(tiers: usize) -> Result<Vec<(f64, Option<f64>)>, String> {
    if tiers == 0 || tiers > LOD_BANDS_M.len() {
        return Err(format!(
            "a building has 1 to {} LOD tiers; the sidecar lists {tiers}",
            LOD_BANDS_M.len()
        ));
    }
    Ok((0..tiers)
        .map(|tier| {
            (
                LOD_BANDS_M[tier],
                (tier + 1 < tiers).then(|| LOD_BANDS_M[tier + 1]),
            )
        })
        .collect())
}

/// R7: a variant id as a display name: `_` as spaces, each word capitalised.
pub(crate) fn scheme_name(id: &str) -> String {
    id.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// R7: one scheme per variant, sorted by id; `materials` holds each tier's one material name.
pub(crate) fn appearance(facts: &Facts, part: &str, materials: &[String]) -> Value {
    let mut variants: Vec<_> = facts.variants.iter().collect();
    variants.sort_by(|a, b| a.id.cmp(&b.id));
    let schemes: Vec<Value> = variants
        .into_iter()
        .map(|variant| {
            json!({
                "id": variant.id,
                "name": scheme_name(&variant.id),
                "overrides": [{
                    "part": part,
                    "materials": materials,
                    "albedo": colour_path(&variant.albedo),
                }],
            })
        })
        .collect();
    json!({
        "default_scheme": facts.default_variant,
        "spawn": "random_scheme",
        "schemes": schemes,
    })
}

/// Where a variant albedo is published inside the asset (R7).
pub(crate) fn colour_path(albedo: &str) -> String {
    format!("colours/{albedo}")
}

/// R8: the centred 4:3 region `[x, y, width, height]` of a preview of `size`. A preview
/// narrower than 4:3 keeps its width and is cropped in height instead.
pub(crate) fn thumbnail_crop(size: [i32; 2]) -> [i32; 4] {
    let [width, height] = size;
    let wide = ((f64::from(height) * 4.0 / 3.0).round() as i32).min(width);
    let high = if wide < width {
        height
    } else {
        ((f64::from(width) * 3.0 / 4.0).round() as i32).min(height)
    };
    [(width - wide) / 2, (height - high) / 2, wide, high]
}

/// Free space between the mesh bounds and each lot edge, through the part position: what
/// the owner has for parking, driveway, loading and hedge (Q3, Q5).
pub(crate) fn free_space(facts: &Facts, lot: [u16; 2], part: [f64; 3]) -> Value {
    let half = lot.map(|cells| f64::from(cells) * f64::from(ZONE_CELL_M) * 0.5);
    let [min, max] = [facts.bounds.min, facts.bounds.max];
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    json!({
        "front_m": round(half[1] - (part[2] + max[2])),
        "back_m": round(part[2] + min[2] + half[1]),
        "left_m": round(part[0] + min[0] + half[0]),
        "right_m": round(half[0] - (part[0] + max[0])),
    })
}

/// The pack identity fields the editor copies into every document from `pack.toml`.
pub(crate) struct PackIdentity<'a> {
    pub(crate) name: &'a str,
    pub(crate) author: &'a str,
    pub(crate) version: &'a str,
    pub(crate) license: &'a str,
}

/// The editor document's `params` for a draft: the editor's New Asset preset for the zone
/// type (`authoring::conversion`), then R1-R9 and the role inputs.
pub(crate) fn params(
    facts: &Facts,
    role: &Role,
    pack: &PackIdentity,
    materials: &[String],
) -> Result<Value, String> {
    let preset = super::super::conversion(&json!({}), &role.zone_type, "")?;
    let mut params = preset["document"].clone();
    let [width, depth] = lot(facts, role.lot_cells)?;
    let part_name = &facts.lods[0].file;
    let position = part_position(facts, depth);
    let lods: Vec<Value> = lod_bands(facts.lods.len())?
        .into_iter()
        .zip(&facts.lods)
        .map(|((min, max), lod)| {
            let mut entry = json!({"file": lod.file, "distance_min_m": min});
            if let Some(max) = max {
                entry["distance_max_m"] = json!(max);
            }
            entry
        })
        .collect();
    let fields = json!({
        "pack_id": role.pack_id,
        "pack_name": pack.name,
        "pack_author": pack.author,
        "pack_version": pack.version,
        "pack_license": pack.license,
        "asset_id": role.asset_id,
        "display_name": role.display_name,
        "thumbnail": THUMBNAIL_FILE,
        "density": role.density,
        "level": role.level,
        "lot_width_cells": width,
        "lot_depth_cells": depth,
        "frontage_forward": [0.0, 0.0, 1.0],
        "window_brightness": WINDOW_BRIGHTNESS,
        "placement_mode": "zoned_private",
        "mesh_parts": [{
            "name": part_name,
            "position": position,
            "rotation_degrees": [0.0, 0.0, 0.0],
            "scale": 1.0,
            "pivot_offset": [0.0, 0.0, 0.0],
            "lods": lods,
        }],
        "anchors": [entrance(position, facts.main_entrance()?)],
        "site_surfaces": [],
        "appearance": appearance(facts, part_name, materials),
    });
    for (key, value) in fields.as_object().into_iter().flatten() {
        params[key] = value.clone();
    }
    if role.zone_type == "residential" {
        let flat = flat_size_m2(facts).ok_or(
            "a residential draft needs dwellings and a storey area that rounds to a flat size",
        )?;
        params["household_capacity"] = json!(facts.dwellings);
        params["flat_size_m2"] = json!(flat);
    } else if let Some(profile) = &role.economy_profile {
        params["economy_profile"] = json!(profile);
    }
    if let Some(set) = &role.asset_set {
        params["asset_set"] = json!(set);
    }
    // The model's lanes on the part that shows it (`PARK-21`): part-local is the `.glb`'s frame,
    // so the R2 position places them with the model. Docks, exits and routes are never drafted.
    if facts.passages.as_ref().is_some_and(|p| !p.is_empty()) {
        params["mesh_parts"][0]["passages"] = json!(facts.part_passages());
    }
    Ok(params)
}
