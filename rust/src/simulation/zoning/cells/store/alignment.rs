// SPDX-License-Identifier: GPL-2.0-only

//! Road-edit-owned straight grid choices, independent of viewport and cell cache residency.

use super::*;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::zoning::cells::generation::straight_frame;
use crate::simulation::zoning::cells::geometry::road_source_precision;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Persistent curb frames and their source address. Empty roads retain these choices too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RoadCellAlignment {
    // Exact f32 bits: first X/Z, last X/Z, width. Heights do not select an XZ grid.
    source: [u32; 5],
    #[serde(skip)]
    nodes: [u32; 2],
    // Directed left (-1), right (+1). A split recovers these through its original endpoints.
    frames: [GridFrame; 2],
}

struct StraightRoad {
    source: [u32; 5],
    nodes: [u32; 2],
    points: Vec<DVec2>,
    bounds: CellBounds,
    precision: f64,
    basis: GridFrame,
}

fn source(edge: &Edge) -> Option<[u32; 5]> {
    let a = edge.physical_geometry.first()?;
    let b = edge.physical_geometry.last()?;
    // SQLite REAL canonicalizes signed zero. Both encodings name the same source point.
    Some([a.x, a.z, b.x, b.z, edge.width].map(|v| if v == 0.0 { 0 } else { v.to_bits() }))
}

impl StraightRoad {
    fn new(edge: &Edge, cell_m: f64) -> Option<Self> {
        if edge.deleted
            || edge.primary_type != TransitType::Road
            || edge.class != EdgeClass::Standard
            || edge.physical_geometry.len() < 2
        {
            return None;
        }
        let points: Vec<_> = edge
            .physical_geometry
            .iter()
            .map(|p| DVec2::new(f64::from(p.x), f64::from(p.z)))
            .collect();
        let bounds = CellBounds::from_points(points.iter().copied());
        let straight_precision = road_source_precision(bounds).max(1e-4);
        let direction = (*points.last()? - points[0]).normalize_or_zero();
        if direction.length_squared() == 0.0
            || points
                .iter()
                .any(|p| (*p - points[0]).perp_dot(direction).abs() > straight_precision)
        {
            return None;
        }
        Some(Self {
            source: source(edge)?,
            nodes: [edge.start_node, edge.end_node],
            points,
            bounds,
            precision: road_source_precision(bounds.expanded(6.0 * cell_m)),
            basis: GridFrame::new(DVec2::ZERO, direction, cell_m)?,
        })
    }

    // Both endpoint representations can round independently. The full centreline must fit
    // one axis within their combined numeric bound, not merely look perpendicular on screen.
    fn fits_basis(&self, basis: GridFrame) -> bool {
        let (u, v) = basis.axes();
        let delta = self.points[self.points.len() - 1] - self.points[0];
        let tangent = if delta.dot(u).abs() > delta.dot(v).abs() {
            u
        } else {
            v
        };
        self.points
            .iter()
            .all(|p| (*p - self.points[0]).perp_dot(tangent).abs() <= 2.0 * self.precision)
    }

    fn continues(&self, previous: RoadCellAlignment) -> bool {
        let values = previous.source.map(|v| f64::from(f32::from_bits(v)));
        let a = DVec2::new(values[0], values[1]);
        let b = DVec2::new(values[2], values[3]);
        let tangent = (b - a).normalize_or_zero();
        tangent.length_squared() > 0.0
            && self
                .bounds
                .intersects(CellBounds::from_points([a, b]).expanded(self.precision))
            && self
                .points
                .iter()
                .all(|p| (*p - a).perp_dot(tangent).abs() <= 2.0 * self.precision)
            && self.fits_basis(previous.frames[0])
    }
}

impl CellStore {
    /// Serializes road choices in stable source order; the save adapter remaps live edge ids.
    pub(crate) fn saved_road_alignments(&self) -> Vec<(usize, RoadCellAlignment)> {
        let mut result: Vec<_> = self
            .road_alignments
            .iter()
            .map(|(&id, &value)| (id, value))
            .collect();
        result.sort_unstable_by_key(|&(id, _)| id);
        result
    }

    /// Validates both curb phases against current geometry before publishing saved authority.
    pub(crate) fn restore_road_alignment(
        &mut self,
        id: usize,
        mut value: RoadCellAlignment,
        graph: &RegionGraph,
        config: &WorldConfig,
    ) -> bool {
        let Some(road) = graph
            .get_edge(id)
            .and_then(|e| StraightRoad::new(e, f64::from(config.zone_cell_m)))
        else {
            return false;
        };
        if self.road_alignments.contains_key(&id)
            || value.source != road.source
            || value
                .frames
                .iter()
                .any(|frame| !frame.is_valid() || frame.cell_m() != f64::from(config.zone_cell_m))
            || value.frames[0].at_origin(DVec2::ZERO) != value.frames[1].at_origin(DVec2::ZERO)
            || !road.fits_basis(value.frames[0])
        {
            return false;
        }
        let delta = road.points[road.points.len() - 1] - road.points[0];
        let (u, v) = value.frames[0].axes();
        let along = usize::from(delta.dot(v).abs() > delta.dot(u).abs());
        let tangent = [u, v][along] * delta.dot([u, v][along]).signum();
        let normal = DVec2::new(tangent.y, -tangent.x);
        let width = f64::from(f32::from_bits(value.source[4])) * 0.5
            + f64::from(crate::config::SIDEWALK_WIDTH);
        for (side, frame) in value.frames.into_iter().enumerate() {
            let point =
                frame.local(road.points[0] + normal * width * if side == 0 { -1.0 } else { 1.0 });
            if (point[1 - along] - point[1 - along].round()).abs() * frame.cell_m() > road.precision
            {
                return false;
            }
        }
        value.nodes = road.nodes;
        self.road_alignments.insert(id, value);
        true
    }

    /// Returns a prepared frame pair only for the exact current source geometry address.
    pub(in crate::simulation::zoning::cells) fn road_alignment(
        &self,
        graph: &RegionGraph,
        id: usize,
    ) -> Option<[GridFrame; 2]> {
        let alignment = self.road_alignments.get(&id)?;
        (source(graph.get_edge(id)?)? == alignment.source).then_some(alignment.frames)
    }

    /// Updates changed roads and one adjacency ring, returning the scope to invalidate.
    /// Work is O((K + A) log K + X) for affected roads, incidences and tested point/source pairs;
    /// retained choices stop traversal at that ring rather than walking the city network.
    pub(crate) fn refresh_road_alignments(
        &mut self,
        graph: &RegionGraph,
        config: &WorldConfig,
        edges: impl IntoIterator<Item = usize>,
    ) -> Vec<usize> {
        let edited: BTreeSet<_> = edges.into_iter().collect();
        let mut scope = edited.clone();
        for &id in &edited {
            if let Some(edge) = graph.get_edge(id) {
                for node in [edge.start_node, edge.end_node] {
                    scope.extend(
                        graph
                            .node_adjacency(graph.get_valid_node(node))
                            .iter()
                            .copied(),
                    );
                }
            }
        }
        let cell_m = f64::from(config.zone_cell_m);
        let roads: BTreeMap<_, _> = scope
            .par_iter()
            .filter_map(|&id| StraightRoad::new(graph.get_edge(id)?, cell_m).map(|road| (id, road)))
            .collect();
        // Old deleted parents still describe splits. Endpoint lookup is local and avoids
        // searching every saved road choice when children receive fresh graph indices.
        let mut old_at_node: HashMap<u32, Vec<(usize, RoadCellAlignment)>> = HashMap::new();
        for &id in &scope {
            if let Some(&alignment) = self.road_alignments.get(&id) {
                for node in alignment.nodes {
                    old_at_node
                        .entry(graph.get_valid_node(node))
                        .or_default()
                        .push((id, alignment));
                }
            }
        }
        let bases = select_bases(&self.road_alignments, graph, &roads, &old_at_node);
        let phases = select_phases(graph, &roads, &old_at_node, &bases);
        for &id in &scope {
            if graph.get_edge(id).is_none_or(|edge| !edge.deleted) && !roads.contains_key(&id) {
                self.road_alignments.remove(&id);
            }
        }
        for (&id, road) in &roads {
            self.road_alignments.insert(
                id,
                RoadCellAlignment {
                    source: road.source,
                    nodes: road.nodes,
                    frames: [phases[&(id, 0)], phases[&(id, 1)]],
                },
            );
        }
        scope.into_iter().collect()
    }
}

// Constraint propagation is ordered; independent road sampling above uses Rayon.
fn select_bases(
    previous: &HashMap<usize, RoadCellAlignment>,
    graph: &RegionGraph,
    roads: &BTreeMap<usize, StraightRoad>,
    old_at_node: &HashMap<u32, Vec<(usize, RoadCellAlignment)>>,
) -> BTreeMap<usize, GridFrame> {
    let mut bases = BTreeMap::new();
    for (&id, road) in roads {
        let prior = previous
            .get(&id)
            .copied()
            .filter(|old| road.continues(*old))
            .or_else(|| {
                road.nodes
                    .iter()
                    .flat_map(|&node| {
                        old_at_node
                            .get(&graph.get_valid_node(node))
                            .into_iter()
                            .flatten()
                    })
                    .filter(|(_, old)| road.continues(*old))
                    .min_by_key(|(id, _)| *id)
                    .map(|(_, old)| *old)
            });
        if let Some(prior) = prior {
            bases.insert(id, prior.frames[0].at_origin(DVec2::ZERO));
        }
    }
    let mut queue: VecDeque<_> = bases.keys().copied().collect();
    // Existing sources seed new connected roads. Any entirely new component starts at
    // its lowest edge id, making import initialization independent of worker/view order.
    let mut seeds = roads.keys().copied();
    loop {
        if queue.is_empty() {
            let Some(seed) = seeds.find(|id| !bases.contains_key(id)) else {
                break;
            };
            bases.insert(seed, roads[&seed].basis);
            queue.push_back(seed);
        }
        while let Some(id) = queue.pop_front() {
            let basis = bases[&id];
            for node in roads[&id].nodes {
                for &neighbor in graph.node_adjacency(graph.get_valid_node(node)) {
                    if let Some(road) = roads.get(&neighbor)
                        && !bases.contains_key(&neighbor)
                        && road.fits_basis(basis)
                    {
                        bases.insert(neighbor, basis);
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }
    bases
}

fn select_phases(
    graph: &RegionGraph,
    roads: &BTreeMap<usize, StraightRoad>,
    old_at_node: &HashMap<u32, Vec<(usize, RoadCellAlignment)>>,
    bases: &BTreeMap<usize, GridFrame>,
) -> BTreeMap<(usize, usize), GridFrame> {
    let raw: BTreeMap<_, _> = roads
        .iter()
        .map(|(&id, _)| {
            (
                id,
                [
                    straight_frame(graph, id, bases[&id], -1.0),
                    straight_frame(graph, id, bases[&id], 1.0),
                ],
            )
        })
        .collect();
    let mut phases = BTreeMap::new();
    for (&id, road) in roads {
        for side in 0..2 {
            let matching = road
                .nodes
                .iter()
                .flat_map(|&node| {
                    old_at_node
                        .get(&graph.get_valid_node(node))
                        .into_iter()
                        .flatten()
                })
                .flat_map(|&(source, old)| old.frames.into_iter().map(move |frame| (source, frame)))
                .filter(|(_, frame)| raw[&id][side].phase_matches(*frame, road.precision))
                .min();
            if let Some((_, frame)) = matching {
                phases.insert((id, side), frame);
            }
        }
    }
    let mut queue: VecDeque<_> = phases.keys().copied().collect();
    let mut seeds = roads.keys().flat_map(|&id| [(id, 0), (id, 1)]);
    loop {
        if queue.is_empty() {
            let Some((seed, side)) = seeds.find(|key| !phases.contains_key(key)) else {
                break;
            };
            phases.insert((seed, side), raw[&seed][side]);
            queue.push_back((seed, side));
        }
        while let Some((id, side)) = queue.pop_front() {
            let frame = phases[&(id, side)];
            for node in roads[&id].nodes {
                for &neighbor in graph.node_adjacency(graph.get_valid_node(node)) {
                    let Some(road) = roads.get(&neighbor) else {
                        continue;
                    };
                    for neighbor_side in 0..2 {
                        if !phases.contains_key(&(neighbor, neighbor_side))
                            && raw[&neighbor][neighbor_side].phase_matches(frame, road.precision)
                        {
                            phases.insert((neighbor, neighbor_side), frame);
                            queue.push_back((neighbor, neighbor_side));
                        }
                    }
                }
            }
        }
    }
    phases
}
