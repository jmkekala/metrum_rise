// SPDX-License-Identifier: GPL-3.0-or-later

//! "Move whole site" (`TOOLS-40`, docs/asset_editor.md "Site layout tools"): one offset for
//! everything on the lot, as one edit, so a layout is re-centred on a resized lot without moving
//! each object by hand. Every mesh part, every access point, every site surface and planting
//! vertex and every loading bay waypoint moves by `[dx, dz]`. A part's passages are part-local
//! and move with it. A Driveway sits on the street edge, so it moves only along that edge. The
//! move is refused, naming the first object, when anything would leave the lot. Pure: the
//! editor's command writes the result into its document. O(parts + anchors + vertices +
//! waypoints), once per edit.

use crate::assets::MeshPart;
use crate::assets::asset::geometry::fits_lot;
use crate::assets::asset::validation::site_anchor_footprint;
use serde_json::{Value, json};

/// The editor's lot fit tolerance for parts (`_mesh_part_fits_in_lot`), metres.
const PART_FIT_TOLERANCE_M: f32 = 0.01;
/// Manifest validation's tolerance for anchors and polygon vertices on the lot edge, metres.
const LOT_EDGE_TOLERANCE_M: f64 = 0.001;

/// The moved values, in document order: what the command writes back, and nothing else.
#[derive(Debug, PartialEq)]
pub(crate) struct MovedSite {
    /// Each mesh part's new `position`.
    pub(crate) parts: Vec<[f64; 3]>,
    /// Each anchor's new `position` and, when it has them, its moved `route_in` and `route_out`.
    pub(crate) anchors: Vec<MovedAnchor>,
    /// Each site surface's (and planting area's) new `vertices`.
    pub(crate) surfaces: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct MovedAnchor {
    pub(crate) position: [f64; 3],
    pub(crate) route_in: Option<Vec<[f64; 2]>>,
    pub(crate) route_out: Option<Vec<[f64; 2]>>,
}

/// Move the editor document's whole site (`params`) by `[dx, dz]` metres. `part_bounds` is six
/// floats per mesh part, its imported mesh-root bounds (minimum, then maximum; a part without
/// them is placed but not fit-checked, as the editor's own fit check does). Values are rounded to
/// 0.1 mm, the position fields' precision.
pub(crate) fn move_site(
    params: &Value,
    part_bounds: &[f32],
    dx: f64,
    dz: f64,
) -> Result<MovedSite, String> {
    if !(dx.is_finite() && dz.is_finite()) {
        return Err("Expected two finite numbers".into());
    }
    let half = ["lot_width_cells", "lot_depth_cells"].map(|key| {
        params[key].as_f64().unwrap_or(1.0) * f64::from(crate::assets::asset::validation::ZONE_CELL_M) * 0.5
    });
    let outside = |what: String| {
        format!(
            "Moving the site by {dx} m, {dz} m would take {what} outside the lot (±{} × ±{} m)",
            half[0], half[1]
        )
    };
    let inside = |x: f64, z: f64| {
        x.abs() <= half[0] + LOT_EDGE_TOLERANCE_M && z.abs() <= half[1] + LOT_EDGE_TOLERANCE_M
    };
    let shift = |point: [f64; 2], by: [f64; 2]| [round(point[0] + by[0]), round(point[1] + by[1])];
    let list = |value: &Value| value.as_array().cloned().unwrap_or_default();

    let mut parts = Vec::new();
    for (index, part) in list(&params["mesh_parts"]).iter().enumerate() {
        let old = triple(&part["position"]);
        let position = [round(old[0] + dx), old[1], round(old[2] + dz)];
        let bounds = part_bounds.get(index * 6..index * 6 + 6).filter(|b| {
            b[3] - b[0] >= 0.001 && b[5] - b[2] >= 0.001
        });
        if let Some(b) = bounds {
            let mut placed = MeshPart::single_lod0("", "");
            placed.position = position.map(|v| v as f32);
            placed.rotation_degrees = triple(&part["rotation_degrees"]).map(|v| v as f32);
            placed.scale = part["scale"].as_f64().unwrap_or(1.0) as f32;
            placed.pivot_offset = part
                .get("pivot_offset")
                .filter(|v| v.is_array())
                .map(|v| triple(v).map(|v| v as f32));
            let plan = placed.plan_bounds([[b[0], b[1], b[2]], [b[3], b[4], b[5]]]);
            let lot = half.map(|v| v as f32);
            if !fits_lot(plan, lot, PART_FIT_TOLERANCE_M) {
                return Err(outside(format!("mesh part '{}'", text(part, "name"))));
            }
        }
        parts.push(position);
    }

    // A Driveway keeps its across-street coordinate on the frontage edge.
    let frontage = triple(&params["frontage_forward"]);
    let along_edge = if frontage[0].abs() > 0.5 { [0.0, dz] } else { [dx, 0.0] };
    let mut anchors = Vec::new();
    for anchor in list(&params["anchors"]) {
        let name = text(&anchor, "name");
        let kind = text(&anchor, "anchor_type");
        let by = if kind == "driveway" { along_edge } else { [dx, dz] };
        let old = triple(&anchor["position"]);
        let moved = shift([old[0], old[2]], by);
        let position = [moved[0], old[1], moved[1]];
        // The access point's own point, then its painted footprint, as validation checks them.
        let what = || format!("{} '{name}'", kind.replace('_', " "));
        if !inside(position[0], position[2]) {
            return Err(outside(what()));
        }
        let footprint = serde_json::from_value::<crate::assets::Anchor>(json!({
            "type": kind, "name": name, "position": position, "forward": anchor["forward"],
            "width_m": anchor["width_m"], "length_m": anchor["length_m"],
        }))
        .ok()
        .and_then(|parsed| site_anchor_footprint(&parsed));
        if footprint
            .into_iter()
            .flatten()
            .any(|[x, z]| !inside(f64::from(x), f64::from(z)))
        {
            return Err(outside(what()));
        }
        let route = |key: &str| -> Result<Option<Vec<[f64; 2]>>, String> {
            let Some(points) = anchor.get(key).and_then(Value::as_array) else {
                return Ok(None);
            };
            let mut moved = Vec::with_capacity(points.len());
            for (index, point) in points.iter().enumerate() {
                let [x, z] = shift([number(&point[0]), number(&point[1])], [dx, dz]);
                if !inside(x, z) {
                    return Err(outside(format!("waypoint {} of {}", index + 1, what())));
                }
                moved.push([x, z]);
            }
            Ok(Some(moved))
        };
        let route_in = route("route_in")?;
        let route_out = route("route_out")?;
        anchors.push(MovedAnchor {
            position,
            route_in,
            route_out,
        });
    }

    let mut surfaces = Vec::new();
    for surface in list(&params["site_surfaces"]) {
        let mut vertices = Vec::new();
        for point in list(&surface["vertices"]) {
            let [x, z] = shift([number(&point[0]), number(&point[1])], [dx, dz]);
            if !inside(x, z) {
                return Err(outside(format!("surface '{}'", text(&surface, "name"))));
            }
            vertices.push([x, z]);
        }
        surfaces.push(vertices);
    }
    Ok(MovedSite {
        parts,
        anchors,
        surfaces,
    })
}

// To the position fields' 0.1 mm, without f64 noise or a `-0`.
fn round(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0 + 0.0
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(0.0)
}

fn triple(value: &Value) -> [f64; 3] {
    [number(&value[0]), number(&value[1]), number(&value[2])]
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The factory's layout on a 5 x 3 lot: a 33 m part, the entrance, a parking spot, the truck
    // gate on the street edge, a loading bay with a way in, and the yard.
    fn factory() -> (Value, Vec<f32>) {
        let params = json!({
            "lot_width_cells": 5, "lot_depth_cells": 3, "frontage_forward": [0, 0, 1],
            "mesh_parts": [{"name": "factory", "position": [-8.5, 0, -2.2031],
                "rotation_degrees": [0, 0, 0], "scale": 1, "pivot_offset": [0, 0, 0],
                "passages": [{"name": "forecourt", "position": [6, 0, 12.8]}]}],
            "anchors": [
                {"anchor_type": "entrance", "name": "main", "position": [-12.6, 0, 9.0938], "forward": [0, 0, 1]},
                {"anchor_type": "parking", "name": "p", "position": [-9.5, 0, 15], "forward": [0, 0, -1], "width_m": 2.5, "length_m": 5},
                {"anchor_type": "driveway", "name": "truck_gate", "position": [20, 0, 15], "forward": [0, 0, -1], "width_m": 6},
                {"anchor_type": "loading_bay", "name": "bay", "position": [16, 0, -1.6], "forward": [-1, 0, 0],
                    "width_m": 3.5, "length_m": 8, "route_in": [[20, 5.4], [13, -1.6]]},
            ],
            "site_surfaces": [{"name": "yard", "vertices": [[-14.11, 15], [-14.11, 9.0938], [23, -4.2], [23, 15]]}],
        });
        (params, vec![-16.4838, 0.0, -12.7969, 16.4838, 15.25, 12.7969])
    }

    #[test]
    fn everything_moves_by_the_offset_and_a_driveway_along_its_edge() {
        let (params, bounds) = factory();
        // The drafted part stands on the back edge, so the factory moves across only.
        let moved = move_site(&params, &bounds, 1.0, 0.0).unwrap();
        assert_eq!(moved.parts, vec![[-7.5, 0.0, -2.2031]]);
        assert_eq!(moved.anchors[0].position, [-11.6, 0.0, 9.0938]);
        assert_eq!(moved.anchors[1].position, [-8.5, 0.0, 15.0]);
        assert_eq!(moved.anchors[2].position, [21.0, 0.0, 15.0]);
        assert_eq!(moved.anchors[3].position, [17.0, 0.0, -1.6]);
        assert_eq!(moved.anchors[3].route_in, Some(vec![[21.0, 5.4], [14.0, -1.6]]));
        assert_eq!(moved.anchors[3].route_out, None);
        assert_eq!(moved.anchors[0].route_in, None);
        assert_eq!(
            moved.surfaces,
            vec![vec![[-13.11, 15.0], [-13.11, 9.0938], [24.0, -4.2], [24.0, 15.0]]]
        );
        // Toward the back, the street gate keeps its place on the street edge (z 15) while the
        // rest moves; the parking spot and the yard on that edge are left out of this case.
        let mut params = params;
        params["anchors"].as_array_mut().unwrap().remove(1);
        params["site_surfaces"] = json!([]);
        let moved = move_site(&params, &[], 0.0, -0.5).unwrap();
        assert_eq!(moved.parts, vec![[-8.5, 0.0, -2.7031]]);
        assert_eq!(moved.anchors[1].position, [20.0, 0.0, 15.0]);
        assert_eq!(moved.anchors[2].position, [16.0, 0.0, -2.1]);
        assert_eq!(moved.anchors[2].route_in, Some(vec![[20.0, 4.9], [13.0, -2.1]]));
    }

    #[test]
    fn a_move_that_takes_anything_off_the_lot_is_refused_by_name() {
        let (params, bounds) = factory();
        // The part's bounds reach x 7.98: 17.1 m more crosses the east edge at 25 m.
        let error = move_site(&params, &bounds, 17.1, 0.0).unwrap_err();
        assert!(error.contains("mesh part 'factory'"), "{error}");
        // Without its imported bounds a part is not fit-checked; the gate then hits the edge.
        let error = move_site(&params, &[], 5.5, 0.0).unwrap_err();
        assert!(error.contains("driveway 'truck_gate'"), "{error}");
        // The parking spot's footprint runs 5 m toward the building; the yard reaches z 15.
        let error = move_site(&params, &bounds, 0.0, 0.5).unwrap_err();
        assert!(error.contains("outside the lot (±25 × ±15 m)"), "{error}");
        let error = move_site(&params, &bounds, 0.0, -11.0).unwrap_err();
        assert!(error.contains("mesh part") || error.contains("bay"), "{error}");
        assert!(move_site(&params, &bounds, f64::NAN, 0.0).is_err());
        // Nothing moves: the same values come back.
        let moved = move_site(&params, &bounds, 0.0, 0.0).unwrap();
        assert_eq!(moved.parts, vec![[-8.5, 0.0, -2.2031]]);
    }
}
