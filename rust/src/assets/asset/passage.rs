// SPDX-License-Identifier: GPL-3.0-or-later

//! Passages through building mesh parts (`PARK-19`): the cut each makes in its part's structure
//! rectangle for the vehicle classes whose clear height it meets, and the checks that keep a
//! passage inside its part. Asset-local geometry only, run once per site derivation and once
//! per pack import; nothing here is read by a tick.

use super::{MeshPart, PASSAGE_EAVES_MARGIN_M, Passage, plan_rectangle};

// How far a passage's turned forward may lean off a lot axis and still run along it.
const LOT_AXIS_EPS: f32 = 1e-3;

/// The passage's forward turned by the part's rotation into asset-local `[x, z]`, snapped to
/// the lot axis it runs along; `None` when it runs along neither. O(1).
pub(crate) fn lot_axis_forward(part: &MeshPart, passage: &Passage) -> Option<[f32; 2]> {
    let [x, _, z] = passage.forward;
    let (sin, cos) = part.rotation_degrees[1].to_radians().sin_cos();
    // Godot's positive yaw, as `MeshPart::local_transform`: +X turns toward -Z.
    let turned = [x * cos + z * sin, -x * sin + z * cos];
    let length = turned[0].hypot(turned[1]);
    if length <= f32::EPSILON {
        return None;
    }
    let [x, z] = turned.map(|v| v / length);
    if (x.abs() - 1.0).abs() <= LOT_AXIS_EPS {
        Some([x.signum(), 0.0])
    } else if (z.abs() - 1.0).abs() <= LOT_AXIS_EPS {
        Some([0.0, z.signum()])
    } else {
        None
    }
}

/// The passage's asset-local `[min, max]` (`[x, z]`) rectangle: `width_m` across and `length_m`
/// deep from the mouth, and with `margins` lengthened by [`PASSAGE_EAVES_MARGIN_M`] in front of
/// the mouth and, when `through`, behind the far end. `None` off a lot axis. O(1).
pub(crate) fn passage_rectangle(
    part: &MeshPart,
    passage: &Passage,
    margins: bool,
) -> Option<[[f32; 2]; 2]> {
    let forward = lot_axis_forward(part, passage)?;
    let [x, _, z] = passage.position;
    let mouth = part
        .local_transform()
        .transform_point3(glam::Vec3::new(x, 0.0, z));
    let (behind_mouth, ahead) = if margins {
        let far = if passage.through {
            PASSAGE_EAVES_MARGIN_M
        } else {
            0.0
        };
        (PASSAGE_EAVES_MARGIN_M, passage.length_m + far)
    } else {
        (0.0, passage.length_m)
    };
    let across = [-forward[1], forward[0]];
    let half = passage.width_m * 0.5;
    let mut rect = [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]];
    for (along, side) in [
        (-behind_mouth, -half),
        (-behind_mouth, half),
        (ahead, -half),
        (ahead, half),
    ] {
        let point = [
            mouth.x + forward[0] * along + across[0] * side,
            mouth.z + forward[1] * along + across[1] * side,
        ];
        rect[0] = [rect[0][0].min(point[0]), rect[0][1].min(point[1])];
        rect[1] = [rect[1][0].max(point[0]), rect[1][1].max(point[1])];
    }
    Some(rect)
}

/// `rect` less `cut`, as at most four axis-aligned pieces sharing edges: the strips left and
/// right of the cut over the whole depth, and those before and behind it between them. Pieces
/// of no area are dropped. O(1).
pub(crate) fn subtract_rectangle(
    rect: [[f32; 2]; 2],
    cut: [[f32; 2]; 2],
    out: &mut Vec<[[f32; 2]; 2]>,
) {
    let [[x0, z0], [x1, z1]] = rect;
    let [[cx0, cz0], [cx1, cz1]] = cut;
    if cx0 >= x1 || cx1 <= x0 || cz0 >= z1 || cz1 <= z0 {
        out.push(rect);
        return;
    }
    let (mx0, mx1) = (cx0.max(x0), cx1.min(x1));
    for piece in [
        [[x0, z0], [mx0, z1]],
        [[mx1, z0], [x1, z1]],
        [[mx0, z0], [mx1, cz0.max(z0)]],
        [[mx0, cz1.min(z1)], [mx1, z1]],
    ] {
        if piece[1][0] > piece[0][0] && piece[1][1] > piece[0][1] {
            out.push(piece);
        }
    }
}

/// Appends the structure rectangles `part` blocks for a vehicle class of `clear_height`: its
/// structure footprint less every passage at least that high. Nothing for ground dressing or a
/// part without bounds. O(P²) for P passages of the part (each cut splits a piece in at most
/// four).
pub(crate) fn class_structure_footprints(
    part: &MeshPart,
    clear_height: f32,
    out: &mut Vec<[[f32; 2]; 2]>,
) {
    let Some(whole) = super::structure_footprint(part) else {
        return;
    };
    let start = out.len();
    out.push(whole);
    for passage in part
        .passages
        .iter()
        .filter(|p| p.clear_height_m >= clear_height)
    {
        let Some(cut) = passage_rectangle(part, passage, true) else {
            continue;
        };
        let pieces: Vec<_> = out.drain(start..).collect();
        for piece in pieces {
            subtract_rectangle(piece, cut, out);
        }
    }
}

/// Refuses a passage that leaves its part's plan rectangle by more than the eaves margin: run
/// once the part's bounds are imported, as a pack load does. O(P).
pub(crate) fn check_passages_in_bounds(part: &MeshPart) -> Result<(), String> {
    if part.passages.is_empty() {
        return Ok(());
    }
    let Some([min, max]) = plan_rectangle(part) else {
        return Ok(());
    };
    for passage in &part.passages {
        let Some([pmin, pmax]) = passage_rectangle(part, passage, false) else {
            continue;
        };
        let outside = (0..2).any(|axis| {
            pmin[axis] < min[axis] - PASSAGE_EAVES_MARGIN_M
                || pmax[axis] > max[axis] + PASSAGE_EAVES_MARGIN_M
        });
        if outside {
            return Err(format!(
                "mesh part '{}' passage '{}' leaves the part's bounds by more than {} m",
                part.name, passage.name, PASSAGE_EAVES_MARGIN_M
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 12.6 m by 16.25 m part, as the coal mine's bunker, with one lane passage along -z.
    fn bunker(through: bool, clear_height_m: f32, length_m: f32) -> MeshPart {
        let mut part = MeshPart::single_lod0("bunker", "bunker.glb");
        part.position = [-5.9, 0.0, 0.175];
        part.imported_bounds = Some([[-6.3, 0.0, -8.125], [6.3, 13.08, 8.125]]);
        part.passages.push(Passage {
            name: "lane".into(),
            position: [-1.9, 0.0, 8.125],
            forward: [0.0, 0.0, -1.0],
            width_m: 5.0,
            length_m,
            clear_height_m,
            through,
        });
        part
    }

    #[test]
    fn a_through_passage_splits_its_rectangle_in_two_and_clears_the_mouth() {
        let part = bunker(true, 4.6, 16.25);
        let mut pieces = Vec::new();
        class_structure_footprints(&part, 4.0, &mut pieces);
        // The part spans x -12.2..0.4 and z -7.95..8.3; the lane x -10.3..-5.3.
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        let (left, right) = (pieces[0], pieces[1]);
        assert!((left[1][0] - -10.3).abs() < 1e-4 && (right[0][0] - -5.3).abs() < 1e-4);
        for piece in pieces {
            assert!((piece[0][1] - -7.95).abs() < 1e-4 && (piece[1][1] - 8.3).abs() < 1e-4);
        }
        // A blind passage leaves the far wall: the lane's two sides and its back.
        let blind = bunker(false, 4.6, 8.0);
        let mut pieces = Vec::new();
        class_structure_footprints(&blind, 4.0, &mut pieces);
        assert_eq!(pieces.len(), 3, "{pieces:?}");
        let back = pieces[2];
        assert!(
            (back[1][1] - 0.3).abs() < 1e-4,
            "the back wall starts 8 m in: {back:?}"
        );
    }

    #[test]
    fn a_low_passage_opens_its_part_for_cars_only() {
        let part = bunker(true, 3.0, 16.25);
        let (mut car, mut freight) = (Vec::new(), Vec::new());
        class_structure_footprints(&part, 2.0, &mut car);
        class_structure_footprints(&part, 4.0, &mut freight);
        assert_eq!(car.len(), 2);
        assert_eq!(
            freight,
            vec![super::super::structure_footprint(&part).unwrap()]
        );
    }

    #[test]
    fn a_passage_must_stay_inside_its_part_and_run_along_a_lot_axis() {
        assert!(check_passages_in_bounds(&bunker(true, 4.6, 16.25)).is_ok());
        // The eaves margin is allowed past the far face, not more.
        assert!(check_passages_in_bounds(&bunker(true, 4.6, 17.25)).is_ok());
        assert!(check_passages_in_bounds(&bunker(true, 4.6, 17.5)).is_err());
        let mut turned = bunker(true, 4.6, 16.25);
        turned.rotation_degrees[1] = 90.0;
        assert_eq!(
            lot_axis_forward(&turned, &turned.passages[0]),
            Some([-1.0, 0.0])
        );
        turned.rotation_degrees[1] = 30.0;
        assert_eq!(lot_axis_forward(&turned, &turned.passages[0]), None);
    }
}
