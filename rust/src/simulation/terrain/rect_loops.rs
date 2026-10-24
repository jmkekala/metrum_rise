// SPDX-License-Identifier: GPL-2.0-only

//! Boundary loops cut to an XZ rectangle (`ROAD-68-D`): paths along a footprint boundary are cut
//! into the runs strictly inside the rectangle, and each run's exit joins the next entry
//! counter-clockwise along the rectangle's boundary, so the footprint stays on the left. The
//! roadbed's terrain loops (`roadbed::terrain_loops`) are built with it. Liang-Barsky in f64; a
//! crossing takes the rectangle's coordinate exactly. `O(S + R log R)` for `S` path segments and
//! `R` runs.

use glam::DVec3;

/// A closed boundary loop, or an open run, inside a rectangle `[min_x, min_z, max_x, max_z]`,
/// with one payload per edge.
#[derive(Clone, Debug, PartialEq)]
pub struct RectLoop<E> {
    /// Vertices in loop order. A loop closes from the last vertex to the first.
    pub points: Vec<DVec3>,
    /// Per edge from `points[i]` to the next: its payload, or `None` along the rectangle.
    pub edges: Vec<Option<E>>,
}

impl<E> Default for RectLoop<E> {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            edges: Vec::new(),
        }
    }
}

impl<E> RectLoop<E> {
    /// With the footprint on the left, a hole runs clockwise and an outer boundary
    /// counter-clockwise in XZ.
    pub fn is_hole(&self) -> bool {
        signed_area_xz(&self.points) < 0.0
    }
}

/// Cuts one boundary path (each vertex with the payload of its edge to the next) into its runs
/// strictly inside the rectangle. A closed path is rotated to start at a vertex that is not
/// strictly inside; one wholly inside is a loop itself.
pub(crate) fn clip_path<E: Copy>(
    path: &mut [(DVec3, Option<E>)],
    closed: bool,
    rect: [f64; 4],
    runs: &mut Vec<RectLoop<E>>,
    loops: &mut Vec<RectLoop<E>>,
) {
    if closed {
        match path.iter().position(|&(point, _)| !inside(rect, point)) {
            Some(first) => path.rotate_left(first),
            None => {
                loops.push(RectLoop {
                    points: path.iter().map(|&(point, _)| point).collect(),
                    edges: path.iter().map(|&(_, edge)| edge).collect(),
                });
                return;
            }
        }
    }
    let count = if closed { path.len() } else { path.len() - 1 };
    let mut run: Option<RectLoop<E>> = None;
    for index in 0..count {
        let (a, edge) = path[index];
        let b = path[(index + 1) % path.len()].0;
        let Some((entry, exit)) = clip_segment(a, b, rect) else {
            continue;
        };
        let current = run.get_or_insert_with(RectLoop::default);
        if !inside(rect, a) {
            current.points.push(entry);
        }
        current.edges.push(edge);
        if inside(rect, b) {
            current.points.push(b);
        } else {
            current.points.push(exit);
            current.edges.push(None);
            runs.extend(run.take());
        }
    }
}

// The end points of the part of `a -> b` inside the rectangle: exactly on the boundary where it
// crosses, `a` or `b` otherwise. `None` when the segment only touches the boundary or runs along
// it.
fn clip_segment(a: DVec3, b: DVec3, rect: [f64; 4]) -> Option<(DVec3, DVec3)> {
    let delta = b - a;
    let planes = [
        (-delta.x, a.x - rect[0]),
        (delta.x, rect[2] - a.x),
        (-delta.z, a.z - rect[1]),
        (delta.z, rect[3] - a.z),
    ];
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    let (mut plane0, mut plane1) = (None, None);
    for (plane, &(p, q)) in planes.iter().enumerate() {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            let t = q / p;
            if t > t0 {
                (t0, plane0) = (t, Some(plane));
            }
        } else {
            let t = q / p;
            if t < t1 {
                (t1, plane1) = (t, Some(plane));
            }
        }
    }
    let at = |t: f64, plane: Option<usize>, end: DVec3| {
        let Some(plane) = plane else {
            return end;
        };
        let mut point = a + delta * t;
        match plane {
            0 => point.x = rect[0],
            1 => point.x = rect[2],
            2 => point.z = rect[1],
            _ => point.z = rect[3],
        }
        point
    };
    let (entry, exit) = (at(t0, plane0, a), at(t1, plane1, b));
    let middle = (entry + exit) * 0.5;
    (t0 < t1 && inside(rect, middle)).then_some((entry, exit))
}

/// Joins every run's exit to the next entry counter-clockwise along the rectangle boundary,
/// passing its corners, so the footprint stays on the left. Corners carry the exit's height.
pub(crate) fn join_runs<E: Copy>(
    runs: Vec<RectLoop<E>>,
    rect: [f64; 4],
    loops: &mut Vec<RectLoop<E>>,
) {
    if runs.is_empty() {
        return;
    }
    let mut entries: Vec<(f64, usize)> = runs
        .iter()
        .enumerate()
        .map(|(run, record)| (perimeter(rect, record.points[0]), run))
        .collect();
    entries.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let [min_x, min_z, max_x, max_z] = rect;
    let lap = 2.0 * (max_x - min_x + max_z - min_z);
    let corners = [
        (min_x, min_z),
        (max_x, min_z),
        (max_x, max_z),
        (min_x, max_z),
    ]
    .map(|(x, z)| (perimeter(rect, DVec3::new(x, 0.0, z)), x, z));
    let mut used = vec![false; runs.len()];
    for first in 0..runs.len() {
        if used[first] {
            continue;
        }
        let mut target = RectLoop::default();
        let mut run = first;
        while !used[run] {
            used[run] = true;
            target.points.extend_from_slice(&runs[run].points);
            target.edges.extend_from_slice(&runs[run].edges);
            let exit = runs[run].points[runs[run].points.len() - 1];
            let from = perimeter(rect, exit);
            let slot = entries.partition_point(|&(at, _)| at < from);
            let (to, successor) = entries[slot % entries.len()];
            let to = if to < from { to + lap } else { to };
            for offset in [0.0, lap] {
                for &(at, x, z) in &corners {
                    if at + offset > from && at + offset < to {
                        target.points.push(DVec3::new(x, exit.y, z));
                        target.edges.push(None);
                    }
                }
            }
            run = successor;
        }
        loops.push(target);
    }
}

// Counter-clockwise distance along the rectangle boundary from its `(min_x, min_z)` corner.
fn perimeter(rect: [f64; 4], point: DVec3) -> f64 {
    let [min_x, min_z, max_x, max_z] = rect;
    let (width, height) = (max_x - min_x, max_z - min_z);
    if point.z == min_z && point.x < max_x {
        point.x - min_x
    } else if point.x == max_x && point.z < max_z {
        width + point.z - min_z
    } else if point.z == max_z && point.x > min_x {
        width + height + max_x - point.x
    } else {
        2.0 * width + height + max_z - point.z
    }
}

/// Strictly inside the rectangle.
pub(crate) fn inside(rect: [f64; 4], point: DVec3) -> bool {
    point.x > rect[0] && point.x < rect[2] && point.z > rect[1] && point.z < rect[3]
}

/// Inside the rectangle or on its boundary.
pub(crate) fn within(rect: [f64; 4], point: DVec3) -> bool {
    point.x >= rect[0] && point.x <= rect[2] && point.z >= rect[1] && point.z <= rect[3]
}

/// Shoelace area in XZ, positive counter-clockwise from +x towards +z.
pub(crate) fn signed_area_xz(points: &[DVec3]) -> f64 {
    let mut twice = 0.0;
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        twice += a.x * b.z - b.x * a.z;
    }
    twice * 0.5
}
