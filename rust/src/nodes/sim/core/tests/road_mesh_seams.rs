// SPDX-License-Identifier: GPL-2.0-only

//! Watertight published road-surface checks shared by the road-family regressions.

use super::SimCore;
use super::road_edit_order::committed;
use super::road_reference_probe::reference_fixtures;
use crate::simulation::terrain::render_frame_centre;
use std::collections::BTreeMap;

type Key = (u64, u64, u32);

fn key((x, z, y): (f64, f64, f32)) -> Key {
    (x.to_bits(), z.to_bits(), y.to_bits())
}

// Every published road chunk face edge in world coordinates (chunk-local f32 plus the chunk's
// frame centre, in f64), with its faces' (layer, chunk); and each face's points with their skirt
// bottom attribute (UV `y`: 1 at a skirt's bottom, 0 on its chain, 0 in other layers).
type RoadEdges = BTreeMap<(Key, Key), Vec<(usize, (i32, (i32, i32)))>>;
type FacePoints = Vec<([(f64, f64, f32); 3], [f32; 3])>;

// The skirt layer's index in `road_edges`.
const SKIRT_LAYER: i32 = 5;

fn road_edges(core: &mut SimCore) -> (RoadEdges, FacePoints) {
    core.precompute_road_mesh_data();
    let roadbed = &core.transit_network.roadbed;
    let mut edges = RoadEdges::new();
    let mut thirds = Vec::new();
    let span = roadbed.chunk_span_m();
    let (origin_x, origin_z) = roadbed.chunk_origin_m();
    for (&chunk, mesh) in &core.cached_road_mesh_chunks {
        // Chunk vertices are local to the chunk's frame centre.
        let (centre_x, centre_z) = (
            f64::from(render_frame_centre(origin_x + chunk.0 as f32 * span, span)),
            f64::from(render_frame_centre(origin_z + chunk.1 as f32 * span, span)),
        );
        for (layer, (vertices, uvs)) in [
            (&mesh.road_vertices, &mesh.road_uvs),
            (&mesh.sidewalk_vertices, &mesh.sidewalk_uvs),
            (&mesh.curb_vertices, &mesh.curb_uvs),
            (&mesh.raised_step_vertices, &mesh.raised_step_uvs),
            (&mesh.concrete_vertices, &mesh.concrete_uvs),
            (&mesh.skirt_vertices, &mesh.skirt_uvs),
        ]
        .into_iter()
        .enumerate()
        {
            let faces = vertices.as_chunks::<3>().0.iter();
            for (face, uvs) in faces.zip(uvs.as_chunks::<3>().0) {
                let points =
                    face.map(|v| (f64::from(v.x) + centre_x, f64::from(v.z) + centre_z, v.y));
                let skirt = layer as i32 == SKIRT_LAYER;
                let index = thirds.len();
                thirds.push((points, uvs.map(|uv| if skirt { uv.y } else { 0.0 })));
                for (p, q) in [(0, 1), (1, 2), (2, 0)] {
                    let (p, q) = (key(points[p]), key(points[q]));
                    edges
                        .entry(if p < q { (p, q) } else { (q, p) })
                        .or_default()
                        .push((index, (layer as i32, chunk)));
                }
            }
        }
    }
    (edges, thirds)
}

fn open_road_edges(core: &mut SimCore) -> Vec<String> {
    open_road_edges_where(core, |_, _, _| false)
}

// As `open_road_edges`, without the single-face edges `exempt` admits by whether they are a
// skirt-layer bottom edge (both ends carry the bottom attribute) and their two world points (x,
// y, z).
pub(super) fn open_road_edges_where(
    core: &mut SimCore,
    exempt: impl Fn(bool, (f64, f64, f64), (f64, f64, f64)) -> bool,
) -> Vec<String> {
    let (edges, thirds) = road_edges(core);
    let roadbed = &core.transit_network.roadbed;
    let mut problems = Vec::new();
    for ((p, q), users) in &edges {
        if users.len() == 2 {
            continue;
        }
        let [a, b] = [p, q].map(|k| (f64::from_bits(k.0), f64::from(f32::from_bits(k.2)), f64::from_bits(k.1)));
        if users.len() > 2 {
            problems.push(format!("edge {a:?}-{b:?} shared by {} faces", users.len()));
            continue;
        }
        let (face, (layer, _)) = users[0];
        let (dx, dz) = (b.0 - a.0, b.2 - a.2);
        let length = dx.hypot(dz);
        // A skirt's bottom edge meets the ground below the road's edge: its two ends carry the
        // bottom attribute (exactly 1 at strip vertices; clip points on the bottom edge
        // interpolate it). Every other edge with one face is open, the top surface's outer edges
        // included, which a skirt must close.
        let (points, attributes) = &thirds[face];
        let bottom =
            |k: &Key| attributes[points.iter().position(|point| key(*point) == *k).unwrap()] > 0.5;
        let skirt_bottom = layer == SKIRT_LAYER && length > 0.0 && bottom(p) && bottom(q);
        if exempt(skirt_bottom, a, b) {
            continue;
        }
        if skirt_bottom {
            // It lies under the footprint's edge: road 5 mm to one side of it, none to the other.
            let road_across = |side: f64| {
                let (x, z) = (
                    (a.0 + b.0) * 0.5 + side * 0.005 * dz / length,
                    (a.2 + b.2) * 0.5 - side * 0.005 * dx / length,
                );
                roadbed.band_at(x as f32, z as f32).is_some()
            };
            if road_across(1.0) != road_across(-1.0) {
                continue;
            }
        }
        problems.push(format!(
            "open edge ({:.4}, {:.4}, {:.4})-({:.4}, {:.4}, {:.4}) length {length:.4} layer/chunk {:?}",
            a.0, a.1, a.2, b.0, b.1, b.2, users[0].1
        ));
    }
    problems
}

#[test]
fn reference_junction_road_chunks_are_watertight() {
    let mut failures = Vec::new();
    for sloped in [false, true] {
        for (name, strokes, order) in reference_fixtures() {
            let mut core = committed(&strokes, &order, sloped);
            let problems = open_road_edges(&mut core);
            if !problems.is_empty() {
                failures.push(format!(
                    "{name}/{sloped}: {} problems\n  {}",
                    problems.len(),
                    problems[..problems.len().min(12)].join("\n  ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
