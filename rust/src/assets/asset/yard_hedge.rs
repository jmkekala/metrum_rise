// SPDX-License-Identifier: GPL-2.0-only

//! Yard hedge contract for building assets and the hedge rows it plans inside the lot.
//!
//! The plan is pure lot geometry in asset-local metres, so the asset editor previews exactly the
//! rows a spawned building lays; the simulation only transforms them into the world.

use serde::Deserialize;

/// Which clipped hedge a building's yard is lined with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YardHedgeKind {
    /// Cotoneaster, 0.9 m tall.
    Low,
    /// Currant, 1.4 m tall.
    Medium,
    /// Spruce, 1.9 m tall.
    Tall,
}

impl YardHedgeKind {
    /// Parses the manifest and editor spelling; `None` for anything else, including `"none"`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "tall" => Some(Self::Tall),
            _ => None,
        }
    }

    /// Manifest and editor spelling.
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::Tall => "tall",
        }
    }

    /// Index of this hedge among the low, medium and tall modules.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One side of a building lot, named as seen from the street looking at the building.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LotEdge {
    /// The street side.
    Front,
    /// The side away from the street.
    Back,
    /// The left side as seen from the street.
    Left,
    /// The right side as seen from the street.
    Right,
}

impl LotEdge {
    /// Every edge, in manifest order.
    pub const ALL: [Self; 4] = [Self::Front, Self::Back, Self::Left, Self::Right];

    /// Parses the manifest and editor spelling.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|edge| edge.name() == name)
    }

    /// Manifest and editor spelling.
    pub fn name(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Authored yard hedge of a building asset (`[building.yard_hedge]`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YardHedge {
    /// Hedge the lot is lined with.
    pub hedge: YardHedgeKind,
    /// Lot edges that carry it; all four when omitted.
    #[serde(default = "all_edges")]
    pub edges: Vec<LotEdge>,
}

fn all_edges() -> Vec<LotEdge> {
    LotEdge::ALL.to_vec()
}

/// The lot a yard hedge is planned in, in asset-local metres: the lot is the rectangle
/// `[-half_width_m, half_width_m] x [-half_depth_m, half_depth_m]` in local X and Z.
pub struct YardLot<'a> {
    /// Half the lot along local X.
    pub half_width_m: f32,
    /// Half the lot along local Z.
    pub half_depth_m: f32,
    /// Asset-local frontage direction (X, Z); snapped to the nearest lot axis.
    pub frontage: [f32; 2],
    /// Authored yard surfaces (driveways, walkways), local `[x, z]` polygons.
    pub surfaces: &'a [Vec<[f32; 2]>],
    /// Main entrance position (local X, Z), which keeps a walkway gap in the front row.
    pub entrance: Option<[f32; 2]>,
    /// The building's walls: each mesh part's footprint as a local `[min, max]` rectangle in
    /// X and Z. A row keeps off them as off a surface, which is also where the game refuses a
    /// plant, so a planned row is never cut in the world.
    pub structures: &'a [[[f32; 2]; 2]],
}

/// The local `[min, max]` X/Z rectangle a mesh part's imported `[min, max]` bounds cover once
/// the part's own transform places them; what [`YardLot::structures`] takes.
pub fn structure_footprint(part: &super::MeshPart) -> Option<[[f32; 2]; 2]> {
    let [min, max] = part.imported_bounds?;
    let transform = part.local_transform();
    let mut rect = [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]];
    for (x, z) in [(min[0], min[2]), (min[0], max[2]), (max[0], max[2]), (max[0], min[2])] {
        let p = transform.transform_point3(glam::Vec3::new(x, 0.0, z));
        rect[0] = [rect[0][0].min(p.x), rect[0][1].min(p.z)];
        rect[1] = [rect[1][0].max(p.x), rect[1][1].max(p.z)];
    }
    Some(rect)
}

/// One straight hedge row in asset-local metres. `join_from` and `join_to` say whether that end
/// lies on a lot corner, where it may join the hedge it meets; an end cut by a gap stays square.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YardHedgeRow {
    /// Start of the row.
    pub from: [f32; 2],
    /// End of the row.
    pub to: [f32; 2],
    /// Whether the start may join a hedge it meets.
    pub join_from: bool,
    /// Whether the end may join a hedge it meets.
    pub join_to: bool,
}

/// How far the front row stands inside the lot, clear of the sidewalk at the lot line.
pub const FRONT_INSET_M: f32 = 0.75;
// Spacing of the clearance samples along a row.
const SAMPLE_M: f32 = 0.25;
// Clearance a hedge keeps from a yard surface it would otherwise grow across: half the widest
// hedge and a little more, so the gap reads as a gap and not as a hedge clipping the paving.
const SURFACE_CLEAR_M: f32 = 0.6;
// Half the walkway gap the front row leaves in front of the main entrance.
const ENTRANCE_GAP_HALF_M: f32 = 0.8;
// Shortest piece of hedge worth laying between two gaps.
const MIN_PIECE_M: f32 = 1.0;

/// Plans the hedge rows of `edges` in `lot`. Front and back rows run across the lot and the
/// side rows from the back to the front row's line, all on the lot line except the front row,
/// which stands `FRONT_INSET_M` inside it; side and back rows on the lot line are where a
/// neighbour's hedge stands too, so adjoining lots share them. Every row is cut where it would
/// cross a yard surface or the building's walls, and the front row also in front of the entrance.
/// O((L / SAMPLE_M) * V) for row length L and V surface and wall vertices.
pub fn plan_yard_hedge(lot: &YardLot<'_>, edges: &[LotEdge]) -> Vec<YardHedgeRow> {
    // Frontage snapped to a lot axis, and the right-hand side as seen from the street.
    let front = if lot.frontage[0].abs() > lot.frontage[1].abs() {
        [lot.frontage[0].signum(), 0.0]
    } else {
        [0.0, if lot.frontage[1] < 0.0 { -1.0 } else { 1.0 }]
    };
    let right = [front[1], -front[0]];
    let half = |axis: [f32; 2]| axis[0].abs() * lot.half_width_m + axis[1].abs() * lot.half_depth_m;
    let (along_front, across) = (half(front), half(right));
    let front_line = along_front - FRONT_INSET_M;
    let at = |f: f32, r: f32| [front[0] * f + right[0] * r, front[1] * f + right[1] * r];
    let mut rows = Vec::new();
    for &edge in LotEdge::ALL.iter().filter(|edge| edges.contains(edge)) {
        // Each row as (start, end, offset along it -> point).
        let (length, point): (f32, Box<dyn Fn(f32) -> [f32; 2]>) = match edge {
            LotEdge::Front => (2.0 * across, Box::new(move |s| at(front_line, s - across))),
            LotEdge::Back => (2.0 * across, Box::new(move |s| at(-along_front, s - across))),
            LotEdge::Left => (front_line + along_front, Box::new(move |s| at(s - along_front, -across))),
            LotEdge::Right => (front_line + along_front, Box::new(move |s| at(s - along_front, across))),
        };
        let entrance_s = (edge == LotEdge::Front)
            .then_some(lot.entrance)
            .flatten()
            .map(|e| e[0] * right[0] + e[1] * right[1] + across);
        let blocked = |s: f32| {
            let p = point(s);
            entrance_s.is_some_and(|e| (s - e).abs() < ENTRANCE_GAP_HALF_M)
                || lot.surfaces.iter().any(|polygon| near_polygon(p, polygon, SURFACE_CLEAR_M))
                || lot.structures.iter().any(|&[a, b]| {
                    near_polygon(p, &[a, [a[0], b[1]], b, [b[0], a[1]]], SURFACE_CLEAR_M)
                })
        };
        let samples = (length / SAMPLE_M).round().max(1.0) as usize;
        let mut start: Option<usize> = None;
        for i in 0..=samples + 1 {
            let open = i <= samples && !blocked(length * i as f32 / samples as f32);
            match (open, start) {
                (true, None) => start = Some(i),
                (false, Some(first)) => {
                    let (s0, s1) = (
                        length * first as f32 / samples as f32,
                        length * (i - 1) as f32 / samples as f32,
                    );
                    if s1 - s0 >= MIN_PIECE_M {
                        rows.push(YardHedgeRow {
                            from: point(s0),
                            to: point(s1),
                            join_from: first == 0,
                            join_to: i - 1 == samples,
                        });
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    rows
}

// Whether `p` lies inside `polygon` or within `clear` of its outline.
fn near_polygon(p: [f32; 2], polygon: &[[f32; 2]], clear: f32) -> bool {
    let mut inside = false;
    let mut near = false;
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
        {
            inside = !inside;
        }
        let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
        let length_sq = ab[0] * ab[0] + ab[1] * ab[1];
        let t = if length_sq > 0.0 {
            ((ap[0] * ab[0] + ap[1] * ab[1]) / length_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (dx, dz) = (ap[0] - ab[0] * t, ap[1] - ab[1] * t);
        near |= dx * dx + dz * dz < clear * clear;
    }
    inside || near
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(surfaces: &[Vec<[f32; 2]>], entrance: Option<[f32; 2]>) -> YardLot<'_> {
        YardLot {
            half_width_m: 10.0,
            half_depth_m: 10.0,
            frontage: [0.0, 1.0],
            surfaces,
            entrance,
            structures: &[],
        }
    }

    #[test]
    fn an_open_lot_is_lined_on_its_lot_line_with_the_front_row_inset() {
        let rows = plan_yard_hedge(&lot(&[], None), &LotEdge::ALL);
        assert_eq!(rows.len(), 4);
        // Front: across the lot, inset from the street, left to right as seen from the street.
        assert_eq!(rows[0].from, [-10.0, 9.25]);
        assert_eq!(rows[0].to, [10.0, 9.25]);
        // Back on the back lot line; sides from it to the front row's line.
        assert_eq!((rows[1].from[1], rows[1].to[1]), (-10.0, -10.0));
        assert_eq!((rows[2].from, rows[2].to), ([-10.0, -10.0], [-10.0, 9.25]));
        assert_eq!((rows[3].from, rows[3].to), ([10.0, -10.0], [10.0, 9.25]));
        assert!(rows.iter().all(|row| row.join_from && row.join_to));
    }

    #[test]
    fn the_front_row_opens_for_the_driveway_and_the_entrance() {
        // A driveway from x 2..6 reaching the lot line, and the door at x -4.
        let driveway = vec![vec![[2.0, 3.0], [6.0, 3.0], [6.0, 10.0], [2.0, 10.0]]];
        let rows = plan_yard_hedge(&lot(&driveway, Some([-4.0, 2.0])), &[LotEdge::Front]);
        // Left piece, the piece between door and driveway, and right piece; cut ends do not join.
        assert_eq!(rows.len(), 3, "{rows:?}");
        let xs: Vec<_> = rows.iter().map(|row| (row.from[0], row.to[0])).collect();
        assert_eq!(xs, vec![(-10.0, -5.0), (-3.0, 1.25), (6.75, 10.0)], "{rows:?}");
        assert_eq!(
            rows.iter().map(|row| (row.join_from, row.join_to)).collect::<Vec<_>>(),
            vec![(true, false), (false, false), (false, true)]
        );
    }

    #[test]
    fn rows_keep_off_the_walls_of_a_house_reaching_the_back_line() {
        // A house whose eaves reach the back lot line (z -10) between x -5 and 5.
        let walls = [[[-5.0, -10.0], [5.0, 2.0]]];
        let lot = YardLot {
            structures: &walls,
            ..lot(&[], None)
        };
        let rows = plan_yard_hedge(&lot, &[LotEdge::Back]);
        let xs: Vec<_> = rows.iter().map(|row| (row.from[0], row.to[0])).collect();
        assert_eq!(xs, vec![(-10.0, -5.75), (5.75, 10.0)], "{rows:?}");
    }
}
