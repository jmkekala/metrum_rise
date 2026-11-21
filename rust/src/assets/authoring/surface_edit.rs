// SPDX-License-Identifier: GPL-3.0-or-later

//! Site surface vertices as numbers (`TOOLS-40`, docs/asset_editor.md "Site layout tools"): one
//! vertex set to `[x, z]`, or the whole outline set to a rectangle by two corners, each checked
//! as a drag is: every vertex inside the lot and the outline one simple polygon. Pure; the
//! editor's document command writes the result. O(vertices²) for the simple polygon check, once
//! per edit.

use crate::assets::asset::geometry::is_valid;

/// Manifest validation's tolerance for polygon vertices on the lot edge, metres.
const LOT_EDGE_TOLERANCE_M: f64 = 0.001;

/// `vertices` with vertex `index` at `point`, kept as given (the field's 0.1 mm), refused when it
/// leaves the lot (`lot_half`, metres) or the outline stops being a simple polygon.
pub(crate) fn set_vertex(
    vertices: &[[f64; 2]],
    index: usize,
    point: [f64; 2],
    lot_half: [f64; 2],
) -> Result<Vec<[f64; 2]>, String> {
    if index >= vertices.len() {
        return Err("Vertex no longer exists".into());
    }
    let mut next = vertices.to_vec();
    next[index] = point;
    checked(next, lot_half)
}

/// The axis-aligned rectangle with opposite corners `a` and `b`, wound as a new surface is, from
/// its `-x, -z` corner; refused when it has no area or leaves the lot.
pub(crate) fn rectangle(a: [f64; 2], b: [f64; 2], lot_half: [f64; 2]) -> Result<Vec<[f64; 2]>, String> {
    let [x0, x1] = [a[0].min(b[0]), a[0].max(b[0])];
    let [z0, z1] = [a[1].min(b[1]), a[1].max(b[1])];
    if x1 - x0 < 0.01 || z1 - z0 < 0.01 {
        return Err("A rectangle needs two corners at least 1 cm apart on both axes".into());
    }
    checked(vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]], lot_half)
}

fn checked(vertices: Vec<[f64; 2]>, lot_half: [f64; 2]) -> Result<Vec<[f64; 2]>, String> {
    if vertices.iter().flatten().any(|v| !v.is_finite()) {
        return Err("Expected finite numbers".into());
    }
    if vertices.iter().any(|&[x, z]| {
        x.abs() > lot_half[0] + LOT_EDGE_TOLERANCE_M || z.abs() > lot_half[1] + LOT_EDGE_TOLERANCE_M
    }) {
        return Err(format!(
            "A surface vertex must stay inside the lot (±{} × ±{} m)",
            lot_half[0], lot_half[1]
        ));
    }
    let points: Vec<[f32; 2]> = vertices.iter().map(|&[x, z]| [x as f32, z as f32]).collect();
    if !is_valid(&points) {
        return Err("A yard needs at least three vertices and a non-intersecting polygon".into());
    }
    Ok(vertices)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOT: [f64; 2] = [25.0, 15.0];

    #[test]
    fn a_vertex_moves_inside_the_lot_and_keeps_the_polygon_simple() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let moved = set_vertex(&square, 2, [12.3456, 9.0001], LOT).unwrap();
        assert_eq!(moved, vec![[0.0, 0.0], [10.0, 0.0], [12.3456, 9.0001], [0.0, 10.0]]);
        // On the lot edge is inside; past it is not.
        assert!(set_vertex(&square, 2, [25.0, 15.0], LOT).is_ok());
        let error = set_vertex(&square, 2, [25.01, 9.0], LOT).unwrap_err();
        assert!(error.contains("inside the lot (±25 × ±15 m)"), "{error}");
        // A vertex dragged across the opposite edge makes a bow tie.
        let error = set_vertex(&square, 2, [5.0, -5.0], LOT).unwrap_err();
        assert!(error.contains("non-intersecting"), "{error}");
        assert!(set_vertex(&square, 4, [1.0, 1.0], LOT).is_err());
        assert!(set_vertex(&square, 1, [f64::NAN, 1.0], LOT).is_err());
    }

    #[test]
    fn a_rectangle_by_two_corners_in_any_order() {
        let expected = vec![[-14.11, -4.2], [23.0, -4.2], [23.0, 15.0], [-14.11, 15.0]];
        assert_eq!(rectangle([-14.11, -4.2], [23.0, 15.0], LOT).unwrap(), expected);
        assert_eq!(rectangle([23.0, -4.2], [-14.11, 15.0], LOT).unwrap(), expected);
        assert!(rectangle([0.0, 0.0], [0.0, 5.0], LOT).unwrap_err().contains("1 cm"));
        assert!(rectangle([0.0, 0.0], [26.0, 5.0], LOT).unwrap_err().contains("inside the lot"));
    }
}
