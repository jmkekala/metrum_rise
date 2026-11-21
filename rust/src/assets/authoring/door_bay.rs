// SPDX-License-Identifier: GPL-3.0-or-later

//! "Loading bay at vehicle door" (`TOOLS-40`, docs/asset_editor.md "Site layout tools"): an
//! authoring helper the author invokes in the editor for an asset with a model draft ledger
//! (`<pack>/model_sources/<asset_id>.json`). It places a loading bay end-on to one of the
//! sidecar's `vehicle` doors, centred on it, at the editor's standard size, its far end just
//! clear of the part's plan bounds on that side. It derives nothing by itself and nothing at
//! runtime: docks stay the site layout's, authored by the owner (freight yards decision 8). Pure;
//! O(doors) to list, O(1) to place, once per edit.

use super::model_draft::sidecar::{DoorKind, read_ledger};
use crate::assets::MeshPart;
use serde_json::{Value, json};
use std::path::Path;

/// The editor's new loading bay size (`create` in the document commands), metres.
pub(crate) const BAY_WIDTH_M: f64 = 3.5;
pub(crate) const BAY_LENGTH_M: f64 = 8.0;
/// The bay's far end keeps at least this much off the part's plan bounds, then rounds outward to
/// the whole centimetre, so it stands clear of the walls and reads cleanly.
const WALL_CLEARANCE_M: f64 = 0.01;

/// The ledger's `vehicle` doors for the editor: `{part, doors: [{id, position, facing,
/// width_m, height_m}]}` with `part` the sidecar's `LOD0` file (the part that shows the model)
/// and door fields in the part's local frame, as the sidecar gives them. `None` when the asset
/// has no ledger entry.
pub(crate) fn vehicle_doors(pack: &Path, asset_id: &str) -> Result<Option<Value>, String> {
    let Some(facts) = read_ledger(pack, asset_id)? else {
        return Ok(None);
    };
    let doors: Vec<Value> = facts
        .doors
        .iter()
        .filter(|door| door.kind == DoorKind::Vehicle)
        .map(|door| {
            json!({"id": door.id, "position": door.position, "facing": door.facing,
                   "width_m": door.width_m, "height_m": door.height_m})
        })
        .collect();
    Ok(Some(json!({"part": facts.lods[0].file, "doors": doors})))
}

/// A loading bay's `position` (the middle of its entry edge) and `forward` (into the bay, toward
/// the door) at a door of `part`: the door's part-local `position` and outward `facing` turned
/// into the lot frame. `bounds` is the part's imported mesh-root bounds; `lot_half` the lot's half
/// width and depth. The door must face along a lot axis once turned. Refused, with the reason,
/// when it does not, or when the bay would leave the lot.
pub(crate) fn bay_at_door(
    part: &MeshPart,
    bounds: [[f32; 3]; 2],
    door_position: [f64; 3],
    door_facing: [f64; 3],
    lot_half: [f64; 2],
) -> Result<([f64; 3], [f64; 3]), String> {
    let transform = part.local_transform();
    let point = transform.transform_point3(glam::Vec3::from_array(door_position.map(|v| v as f32)));
    let facing = transform.transform_vector3(glam::Vec3::from_array(door_facing.map(|v| v as f32)));
    let facing = glam::Vec2::new(facing.x, facing.z).normalize_or_zero();
    // The lot axis the door faces along: 0 for x, 1 for z, with its sign.
    let (axis, sign) = if facing.x.abs() > 0.999 {
        (0, facing.x.signum())
    } else if facing.y.abs() > 0.999 {
        (1, facing.y.signum())
    } else {
        return Err("The door does not face along a lot axis: turn the part a quarter turn".into());
    };
    let (axis, sign) = (axis, f64::from(sign));
    let plan = part.plan_bounds(bounds);
    // The part's outermost extent on the door's side, measured outward along its facing.
    let extent = f64::from(if sign > 0.0 { plan[1][axis] } else { -plan[0][axis] });
    // Less a hair, so a value already on a centimetre does not round up for float noise.
    let far = ((extent + WALL_CLEARANCE_M) * 100.0 - 1e-6).ceil() / 100.0;
    let door = [f64::from(point.x), f64::from(point.z)];
    let mut entry = door;
    entry[axis] = sign * (far + BAY_LENGTH_M);
    let round = |v: f64| (v * 10_000.0).round() / 10_000.0 + 0.0;
    let position = [round(entry[0]), 0.0, round(entry[1])];
    let mut forward = [0.0; 3];
    forward[axis * 2] = -sign;
    // The bay's rectangle: its entry edge across the door's axis, its far end at `far`.
    let across = 1 - axis;
    let corners = [entry[across] - BAY_WIDTH_M * 0.5, entry[across] + BAY_WIDTH_M * 0.5];
    let lot_edge = 0.001;
    if (far + BAY_LENGTH_M) > lot_half[axis] + lot_edge
        || corners.iter().any(|c| c.abs() > lot_half[across] + lot_edge)
    {
        return Err(format!(
            "A {BAY_WIDTH_M} × {BAY_LENGTH_M} m bay at this door would leave the lot (±{} × ±{} m): enlarge the lot or move the part",
            lot_half[0], lot_half[1]
        ));
    }
    Ok((position, forward))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The machinery factory: its part at [-8.5, 0, -2.2031], LOD0 bounds ±16.4838 × ±12.7969,
    // on a 5 × 3 lot; its `loading` door on the east wall at part-local [16.39, 0, 0.6302].
    fn factory() -> (MeshPart, [[f32; 3]; 2]) {
        let mut part = MeshPart::single_lod0("factory", "factory_lod0.glb");
        part.position = [-8.5, 0.0, -2.2031];
        (part, [[-16.4838, 0.0, -12.7969], [16.4838, 15.25, 12.7969]])
    }

    #[test]
    fn the_factorys_bay_stands_end_on_to_its_loading_door() {
        let (part, bounds) = factory();
        let (position, forward) =
            bay_at_door(&part, bounds, [16.39, 0.0, 0.6302], [1.0, 0.0, 0.0], [25.0, 15.0]).unwrap();
        // The hand-placed bay: entry edge [16, 0, -1.6] facing -X. The door's exact centre is
        // z -1.5729; the far end clears the bounds (x 7.9838) at 8.00.
        assert_eq!(position, [16.0, 0.0, -1.5729]);
        assert_eq!(forward, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_door_on_each_side_gets_its_bay_just_clear_of_that_side() {
        // A 10 x 6 m block at the lot's middle, doors on each wall's centre line.
        let mut part = MeshPart::single_lod0("block", "block_lod0.glb");
        part.position = [1.0, 0.0, -2.0];
        let bounds = [[-5.0, 0.0, -3.0], [5.0, 4.0, 3.0]];
        let lot = [25.0, 15.0];
        let cases = [
            // (door, facing, entry edge, forward)
            ([5.0, 0.0, 0.5], [1.0, 0.0, 0.0], [6.01 + 8.0, 0.0, -1.5], [-1.0, 0.0, 0.0]),
            ([-5.0, 0.0, 0.5], [-1.0, 0.0, 0.0], [-4.01 - 8.0, 0.0, -1.5], [1.0, 0.0, 0.0]),
            ([1.5, 0.0, 3.0], [0.0, 0.0, 1.0], [2.5, 0.0, 1.01 + 8.0], [0.0, 0.0, -1.0]),
            ([1.5, 0.0, -3.0], [0.0, 0.0, -1.0], [2.5, 0.0, -5.01 - 8.0], [0.0, 0.0, 1.0]),
        ];
        for (door, facing, entry, forward) in cases {
            let (position, toward) = bay_at_door(&part, bounds, door, facing, lot).unwrap();
            let close = (0..3).all(|i| (position[i] - entry[i]).abs() < 1e-9);
            assert!(close, "{door:?}: {position:?}, expected {entry:?}");
            assert_eq!(toward, forward, "{door:?}");
        }
        // Turned a quarter turn, the east door faces the street (+Z) and its bay follows.
        part.rotation_degrees = [0.0, -90.0, 0.0];
        let (position, toward) =
            bay_at_door(&part, bounds, [5.0, 0.0, 0.5], [1.0, 0.0, 0.0], lot).unwrap();
        assert_eq!(toward, [0.0, 0.0, -1.0]);
        assert!((position[0] - 0.5).abs() < 1e-4 && (position[2] - (3.01 + 8.0)).abs() < 1e-4, "{position:?}");
    }

    #[test]
    fn a_bay_off_the_lot_or_a_door_off_the_axes_is_refused() {
        let (part, bounds) = factory();
        let error =
            bay_at_door(&part, bounds, [16.39, 0.0, 0.6302], [1.0, 0.0, 0.0], [12.0, 15.0]).unwrap_err();
        assert!(error.contains("would leave the lot"), "{error}");
        let diagonal = std::f64::consts::FRAC_1_SQRT_2;
        let error =
            bay_at_door(&part, bounds, [16.39, 0.0, 0.6302], [diagonal, 0.0, diagonal], [25.0, 15.0])
                .unwrap_err();
        assert!(error.contains("lot axis"), "{error}");
    }
}
