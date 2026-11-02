// SPDX-License-Identifier: GPL-2.0-only

//! `ROAD-99`: grade separation by structure, not by height. Two roads meet only where both are
//! grounded; a bridge or a tunnel's bore crosses other roads without a node, and a road passes
//! under a bridge only with the vehicle clearance under its deck.

use super::*;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::interaction::find_intersection_2d;
use crate::simulation::network::types::EdgeClass;
use std::collections::BTreeMap;

// The profile a crossing's heights are read on: the physical profile when present.
fn profile(edge: &Edge) -> &[Vector3] {
    if edge.physical_geometry.len() >= 2 {
        &edge.physical_geometry
    } else {
        &edge.geometry
    }
}

fn live_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

// What a graph holds that only a height difference separated before `ROAD-99`: every crossing in
// plan of two live road edges with no node there, by class pair, with the height between the two
// profiles at the crossing; and the junctions standing on a bridge's deck (a node with two bridge
// arms and a standard one, made by a ground road the bridge's deck pinned). Each edge's
// candidates come from the edge R-tree: `O(E log E + crossings)`.
#[derive(Default)]
struct HeightOnlyScan {
    // (class, class) -> heights between the profiles at each crossing.
    crossings: BTreeMap<(u8, u8), Vec<f32>>,
    junctions_on_decks: usize,
}

fn scan_height_only(graph: &RegionGraph) -> HeightOnlyScan {
    let mut scan = HeightOnlyScan::default();
    let class_id = |class: EdgeClass| class as u8;
    for (id, edge) in graph.edges().iter().enumerate() {
        if !live_surface(edge) {
            continue;
        }
        let a = profile(edge);
        let (lo, hi) = a.iter().fold(
            (Vector3::splat(f32::MAX), Vector3::splat(f32::MIN)),
            |(lo, hi), p| {
                (
                    Vector3::new(lo.x.min(p.x), 0.0, lo.z.min(p.z)),
                    Vector3::new(hi.x.max(p.x), 0.0, hi.z.max(p.z)),
                )
            },
        );
        let mut others = graph.get_edges_near_aabb(lo, hi);
        others.sort_unstable();
        let ends = |edge: &Edge| {
            [edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node))
        };
        for other_id in others.into_iter().filter(|&other| other > id) {
            let other = graph.edge(other_id);
            if !live_surface(other) {
                continue;
            }
            let shared: Vec<Vector3> = ends(edge)
                .into_iter()
                .filter(|node| ends(other).contains(node))
                .map(|node| graph.node(node).pos)
                .collect();
            let b = profile(other);
            let mut seen: Vec<Vector3> = Vec::new();
            for sa in a.windows(2) {
                for sb in b.windows(2) {
                    let Some((t, u)) = find_intersection_2d(sa[0], sa[1], sb[0], sb[1]) else {
                        continue;
                    };
                    let (pa, pb) = (sa[0].lerp(sa[1], t), sb[0].lerp(sb[1], u));
                    let near = |p: &Vector3, q: Vector3| (p.x - q.x).hypot(p.z - q.z) < 1.0;
                    if shared.iter().any(|node| near(node, pa)) || seen.iter().any(|q| near(q, pa))
                    {
                        continue;
                    }
                    seen.push(pa);
                    let pair = (class_id(edge.class), class_id(other.class));
                    let key = (pair.0.min(pair.1), pair.0.max(pair.1));
                    scan.crossings.entry(key).or_default().push((pa.y - pb.y).abs());
                }
            }
        }
    }
    for node in 0..graph.node_adjacency_count() as u32 {
        if graph.get_valid_node(node) != node {
            continue;
        }
        let arms = graph
            .node_adjacency(node)
            .iter()
            .map(|&edge| graph.edge(edge))
            .filter(|edge| live_surface(edge));
        let bridges = arms.clone().filter(|edge| edge.class == EdgeClass::Bridge).count();
        let ground = arms.filter(|edge| edge.class == EdgeClass::Standard).count();
        if bridges >= 2 && ground >= 1 {
            scan.junctions_on_decks += 1;
        }
    }
    scan
}

fn scan_report(name: &str, graph: &RegionGraph) -> String {
    let scan = scan_height_only(graph);
    let live = graph.edges().iter().filter(|edge| live_surface(edge));
    let count = |class: EdgeClass| live.clone().filter(|edge| edge.class == class).count();
    let mut line = format!(
        "ROAD99 SCAN {name}: edges {} bridges {} tunnels {}; junctions on a bridge deck {}",
        live.clone().count(),
        count(EdgeClass::Bridge),
        count(EdgeClass::Tunnel),
        scan.junctions_on_decks,
    );
    let names = ["standard", "bridge", "tunnel"];
    for ((a, b), heights) in &scan.crossings {
        let within = |lo: f32, hi: f32| heights.iter().filter(|&&h| h >= lo && h < hi).count();
        let (least, most) = heights
            .iter()
            .fold((f32::INFINITY, 0.0_f32), |(lo, hi), &h| (lo.min(h), hi.max(h)));
        line += &format!(
            "; {} x {} crossings without a node {} (under 4.5 m {}, 4.5-4.85 m {}, 4.85 m or more \
             {}; least {least:.2} m, most {most:.2} m)",
            names[*a as usize],
            names[*b as usize],
            heights.len(),
            within(0.0, 4.5),
            within(4.5, 4.85),
            within(4.85, f32::INFINITY),
        );
    }
    if scan.crossings.is_empty() {
        line += "; no crossing without a node";
    }
    line
}

// Before `ROAD-99` lands (its plan, step 1): what the reference save and the user's dumps
// (`METRUM_ROAD99_DUMPS`, `;`-separated copies at the current save format) hold that only height
// separated, by class pair and the height between the profiles. Nothing repairs these on load;
// they change only when one of their roads is redrawn. Read-only; ignored because the dumps are
// not in the repository.
#[test]
#[ignore = "scan: METRUM_ROAD99_DUMPS"]
fn scan_crossings_separated_by_height_alone() {
    let mut sources = vec![(
        "kuopio".to_owned(),
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite"
        )
        .to_owned(),
    )];
    sources.extend(
        std::env::var("METRUM_ROAD99_DUMPS")
            .unwrap_or_default()
            .split(';')
            .filter(|path| !path.is_empty())
            .enumerate()
            .map(|(index, path)| (format!("dump {index}"), path.to_owned())),
    );
    for (name, path) in sources {
        let mut core = test_core();
        core.load_game_internal(&path).expect("load the save");
        println!("{}", scan_report(&name, &core.region_graph));
    }
}
