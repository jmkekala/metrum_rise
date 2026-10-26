// SPDX-License-Identifier: GPL-2.0-only

//! Road-clip query and road-loop variant export helpers.

use super::super::super::*;

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::roadbed::{
    ChainSegment, PieceId, RoadbedSystem, TerrainLoop, TerrainLoopError, terrain_loops,
};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::cdt::{
    TerrainCdtRoadLoopSourceEdge, TerrainCdtRoadPiece, TerrainCdtRoadSupport,
};

// Every road loop of one query is in one footprint group: holes (closed blocks) and islands
// inside them classify by containment parity within the group. Site groups set the top bit.
const ROAD_FOOTPRINT_GROUP_ID: u64 = 0x4000_0000_0000_0000;

impl SimulationNode {
    // Building-site loops only: terrain builds take their road loops per CDT tile
    // (`terrain_cdt_road_tile_loops`).
    pub(in crate::nodes::simulation_node) fn road_clip_loop_query_for_snapshot(
        sites: &BuildingSiteTerrainSnapshot,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> RoadClipLoopQuery {
        let site_loops = sites.terrain_cdt_site_loops_for_world_bounds(min_x, min_z, max_x, max_z);
        let site_loop_count = site_loops.len();
        RoadClipLoopQuery {
            cdt_road_loops: site_loops,
            source_count: site_loop_count,
            road_source_count: 0,
            road_loop_count: 0,
            site_loop_count,
            clip_error_label: None,
        }
    }

    /// The roadbed's terrain loops inside `rect` as CDT road loops: vertices and heights are the
    /// chain vertices, and every chain edge carries a `RoadBoundary` source. Loops, their
    /// rotation and their sources are canonical in the footprint inside `rect` alone
    /// (`ROAD-68-E`): each loop starts at its least vertex by coordinate bits, loops are ordered
    /// by that vertex, and a source names the run of consecutive edges of one piece chain it lies
    /// on, runs numbered by the bits of their first two vertices and edges by their place in the
    /// run, keeping only the piece's kind and support. A tile whose footprint an edit leaves
    /// unchanged so gets the same CDT input bit for bit even when the edit renumbers edges or
    /// chains (a split trunk's far half), and its window is reused.
    /// `O(E log E)` for `E` loop edges beyond [`terrain_loops`].
    pub(in crate::nodes::simulation_node) fn road_terrain_loops(
        graph: &RegionGraph,
        roadbed: &RoadbedSystem,
        rect: [f64; 4],
    ) -> Result<Vec<TerrainCdtRoadLoop>, TerrainLoopError> {
        let mut loops = terrain_loops(roadbed, graph, rect)?;
        for terrain_loop in &mut loops {
            let first = (0..terrain_loop.points.len())
                .min_by_key(|&vertex| vertex_bits(terrain_loop.points[vertex]))
                .unwrap_or(0);
            terrain_loop.points.rotate_left(first);
            terrain_loop.edges.rotate_left(first);
        }
        loops.sort_by_key(|terrain_loop| terrain_loop.points.first().copied().map(vertex_bits));
        let labels = road_run_labels(&loops);
        Ok(loops
            .iter()
            .zip(labels)
            .enumerate()
            .map(|(index, (terrain_loop, labels))| {
                Self::road_terrain_loop(graph, index, terrain_loop, &labels)
            })
            .collect())
    }

    fn road_terrain_loop(
        graph: &RegionGraph,
        index: usize,
        terrain_loop: &TerrainLoop,
        labels: &[Option<(u32, u32)>],
    ) -> TerrainCdtRoadLoop {
        let vertex = |point: crate::simulation::network::roadbed::RoadVec3| {
            TerrainCdtVertex::new(point.x, point.y as f32, point.z)
        };
        let count = terrain_loop.points.len();
        let source_edges = terrain_loop
            .edges
            .iter()
            .zip(labels)
            .enumerate()
            .filter_map(|(edge, (segment, label))| {
                let (segment, (run, place)) = ((*segment)?, (*label)?);
                // A wall run across a structure mouth (`TERRAIN-07-C`) is a structure source,
                // whose retaining-wall policy closes the cut at the abutment or portal.
                let structure = |grounded: bool| {
                    if grounded && !segment.wall {
                        TerrainCdtRoadSupport::Grounded
                    } else {
                        TerrainCdtRoadSupport::Structure
                    }
                };
                let (piece, support) = match segment.piece {
                    PieceId::Span(edge_idx) => (
                        TerrainCdtRoadPiece::Span(u64::from(run)),
                        structure(graph.edge(edge_idx).class == EdgeClass::Standard),
                    ),
                    PieceId::Node(_) => (TerrainCdtRoadPiece::Node(run), structure(true)),
                };
                Some(TerrainCdtRoadLoopSourceEdge {
                    start: vertex(terrain_loop.points[edge]),
                    end: vertex(terrain_loop.points[(edge + 1) % count]),
                    source: TerrainCdtRoadBoundarySource::RoadBoundary {
                        piece,
                        chain: 0,
                        segment: place,
                        support,
                    },
                })
            })
            .collect();
        TerrainCdtRoadLoop::new_with_source_edges_and_topology(
            ROAD_FOOTPRINT_GROUP_ID | index as u64,
            ROAD_FOOTPRINT_GROUP_ID,
            u32::try_from(index).unwrap_or(u32::MAX),
            terrain_loop.is_hole(),
            terrain_loop.points.iter().copied().map(vertex).collect(),
            source_edges,
        )
    }

    pub(in crate::nodes::simulation_node) fn road_clip_query_requires_road_clipping(
        road_clip_query: &RoadClipLoopQuery,
        patch_is_known_road_locked: bool,
    ) -> bool {
        patch_is_known_road_locked
            || road_clip_query.road_source_count > 0
            || road_clip_query.road_loop_count > 0
            || road_clip_query.clip_error_label.is_some()
    }

    pub(in crate::nodes::simulation_node) fn append_road_clip_status(
        dict: &mut VarDictionary,
        road_clip_query: &RoadClipLoopQuery,
    ) {
        let (status, error, source_count) = Self::road_clip_status_values(road_clip_query);
        dict.set("road_clip_status", GString::from(status));
        dict.set("road_clip_error", GString::from(error));
        dict.set("road_clip_source_count", source_count);
        dict.set(
            "road_clip_road_source_count",
            i64::try_from(road_clip_query.road_source_count).unwrap_or(0),
        );
        dict.set(
            "road_clip_road_loop_count",
            i64::try_from(road_clip_query.road_loop_count).unwrap_or(0),
        );
        dict.set(
            "road_clip_site_loop_count",
            i64::try_from(road_clip_query.site_loop_count).unwrap_or(0),
        );
    }

    pub(in crate::nodes::simulation_node) fn road_clip_status_values(
        road_clip_query: &RoadClipLoopQuery,
    ) -> (&'static str, &'static str, i64) {
        let (status, error) = if let Some(error_label) = road_clip_query.clip_error_label {
            ("failed", error_label)
        } else {
            ("ok", "none")
        };
        (
            status,
            error,
            i64::try_from(road_clip_query.source_count).unwrap_or(0),
        )
    }
}

// A vertex's coordinate bits, X then Z then height: a total order that names a point exactly.
fn vertex_bits(point: crate::simulation::network::roadbed::RoadVec3) -> (u64, u64, u64) {
    (point.x.to_bits(), point.z.to_bits(), point.y.to_bits())
}

// Per loop edge, its run number and its place in the run, `None` along the rectangle. A run is
// a maximal cyclic sequence of consecutive edges on one piece chain; one that closes on itself
// starts at the loop's first vertex. Runs are numbered in the order of their first two
// vertices' bits, so the labels depend on the footprint inside the rectangle, not on piece or
// segment numbers (`ROAD-68-E`).
fn road_run_labels(loops: &[TerrainLoop]) -> Vec<Vec<Option<(u32, u32)>>> {
    let same_chain = |a: Option<ChainSegment>, b: Option<ChainSegment>| match (a, b) {
        (Some(a), Some(b)) => a.piece == b.piece && a.chain == b.chain && a.wall == b.wall,
        _ => false,
    };
    // (first two vertices, loop, first edge, length)
    let mut runs = Vec::new();
    for (loop_index, terrain_loop) in loops.iter().enumerate() {
        let edges = &terrain_loop.edges;
        let count = edges.len();
        let mut starts = (0..count)
            .filter(|&edge| {
                edges[edge].is_some() && !same_chain(edges[(edge + count - 1) % count], edges[edge])
            })
            .peekable();
        let closed_run = starts.peek().is_none() && edges.iter().any(Option::is_some);
        for start in starts.chain(closed_run.then_some(0)) {
            let length = (1..count)
                .take_while(|&step| same_chain(edges[start], edges[(start + step) % count]))
                .count()
                + 1;
            let key = (
                vertex_bits(terrain_loop.points[start]),
                vertex_bits(terrain_loop.points[(start + 1) % count]),
            );
            runs.push((key, loop_index, start, length));
        }
    }
    runs.sort_unstable();
    let mut labels: Vec<Vec<Option<(u32, u32)>>> = loops
        .iter()
        .map(|terrain_loop| vec![None; terrain_loop.edges.len()])
        .collect();
    for (run, &(_, loop_index, start, length)) in runs.iter().enumerate() {
        let count = labels[loop_index].len();
        for place in 0..length {
            labels[loop_index][(start + place) % count] = Some((run as u32, place as u32));
        }
    }
    labels
}
