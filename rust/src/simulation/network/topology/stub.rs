// SPDX-License-Identifier: GPL-2.0-only

//! Short stubs go (`ROAD-118` round JS3): a road end too close to the junction an edit makes
//! moves into that junction, and the stub between them goes, so the junction becomes a bend.
//! [`remove_stub_graph`] changes the validation excerpt; [`migrate_stub_dependents`] moves the
//! dependents on the live graph when the stub was the head of a road that keeps its id.

use super::RoadSplitDependentsUndo;
use crate::simulation::network::graph::RegionGraph;

/// What [`remove_stub_graph`] did (`ROAD-118` round JS3), for the plan and the dependents.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StubRemoval {
    /// The road end, now an alias of `junction` and out of the node index.
    pub(crate) end: u32,
    /// The junction the stub reached, where its road now ends.
    pub(crate) junction: u32,
    /// The edge the stub came from: the deleted stub itself, or the split road whose head it was,
    /// which keeps its id on the rest (`trim`).
    pub(crate) from: usize,
    /// The slot the removal deleted: the stub, or the split's second half that `from` took.
    pub(crate) dropped: usize,
    /// The stub's physical length, metres.
    pub(crate) stub_m: f32,
    /// `from`'s physical length before the edit: the stub, or the stub and the rest.
    pub(crate) from_length_m: f32,
    /// Whether `from` keeps a rest beyond the stub.
    pub(crate) trim: bool,
}

/// Removes the stub `stub` from road end `end` to the junction at its other end (`ROAD-118`
/// round JS3) on a graph without turn rules (the validation excerpt): the road end becomes an
/// alias of the junction and leaves the node index. A stub that is the head of a split road
/// (`rest`: the split's second half, from the junction on) is taken off it instead of deleted:
/// the road keeps its id on the rest and the second half's slot is deleted, its crossing
/// overrides moved back to the road, as a dissolved joint's head ([`super::dissolve_joint_graph`]).
/// `None`, with nothing changed, when `end` has another arm or the shapes do not match.
/// `O(points + degree)`.
pub(crate) fn remove_stub_graph(
    graph: &mut RegionGraph,
    end: u32,
    stub: usize,
    rest: Option<usize>,
) -> Option<StubRemoval> {
    let live = |graph: &RegionGraph, id: usize| graph.get_edge(id).is_some_and(|e| !e.deleted);
    if !live(graph, stub)
        || graph
            .node_adjacency(end)
            .iter()
            .any(|&id| id != stub && live(graph, id))
    {
        return None;
    }
    let stub_edge = graph.edge(stub);
    let stub_starts = stub_edge.start_node == end;
    let junction = if stub_starts {
        stub_edge.end_node
    } else {
        stub_edge.start_node
    };
    let stub_m = stub_edge.physical_length;
    if junction == end {
        return None;
    }
    if let Some(rest) = rest {
        let rest_edge = graph.get_edge(rest).filter(|_| live(graph, rest))?;
        if !stub_starts || rest_edge.start_node != junction {
            return None;
        }
    }
    for id in [Some(stub), rest].into_iter().flatten() {
        graph.remove_from_spatial_index(id);
    }
    let (from, dropped, from_length_m) = match rest {
        Some(rest) => {
            // The road keeps its id on the rest: the second half's record moves into its slot.
            let rest_record = graph.edges[rest].clone();
            let far = rest_record.end_node;
            let from_length_m = stub_m + rest_record.physical_length;
            graph.edges[stub] = rest_record;
            graph.edges[rest].deleted = true;
            let far_arms = &mut graph.adjacency[far as usize];
            far_arms.retain(|&id| id != rest);
            far_arms.push(stub);
            far_arms.sort_unstable();
            graph.adjacency[junction as usize].retain(|&id| id != rest);
            let overrides = &mut graph.nodes[far as usize].crosswalk_overrides;
            if let Some(marked) = overrides.remove(&rest) {
                overrides.insert(stub, marked);
            }
            // The junction's arm on the slot is now the rest's, with the rest's crossing.
            let overrides = &mut graph.nodes[junction as usize].crosswalk_overrides;
            overrides.remove(&stub);
            if let Some(marked) = overrides.remove(&rest) {
                overrides.insert(stub, marked);
            }
            graph.add_to_spatial_index(stub);
            (stub, rest, from_length_m)
        }
        None => {
            graph.edges[stub].deleted = true;
            graph.adjacency[junction as usize].retain(|&id| id != stub);
            graph.nodes[junction as usize]
                .crosswalk_overrides
                .remove(&stub);
            (stub, stub, stub_m)
        }
    };
    graph.adjacency[end as usize].clear();
    graph.remove_node_from_spatial_index(end, graph.node(end).pos);
    graph.node_aliases.insert(end, junction);
    Some(StubRemoval {
        end,
        junction,
        from,
        dropped,
        stub_m,
        from_length_m,
        trim: rest.is_some(),
    })
}

/// Moves the dependents of a removed stub that was a road's head (`ROAD-118` round JS3),
/// journaled like a split's: the buildings beyond it keep their road, their stations less the
/// stub ([`BuildingAllocator::migrate_stub_frontages`](crate::simulation::buildings::allocator::BuildingAllocator::migrate_stub_frontages)),
/// and the road's occupancy columns lose the stub's whole cells. No building fronts the stub
/// (the plan's guard). A deleted stub's dependents follow its split, and a stroke's own stub has
/// none, so only a `trim` moves anything. `O(the road's buildings + occupancy cells)`, plus the
/// chunk candidates' test.
pub(in crate::simulation::network) fn migrate_stub_dependents(
    graph: &RegionGraph,
    removal: &StubRemoval,
    zoning: &crate::simulation::zoning::ZoningSystem,
    allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
    journal: &mut Option<Vec<RoadSplitDependentsUndo>>,
) {
    if !removal.trim {
        return;
    }
    let cell_m = zoning.config.zone_cell_m;
    let edge = removal.from;
    let occupancy = allocator.edge_occupancy.get(&edge).cloned();
    let mut buildings = Vec::new();
    allocator.migrate_stub_frontages(
        graph,
        edge,
        removal.stub_m,
        removal.from_length_m,
        cell_m,
        &mut buildings,
    );
    if let Some(cells) = allocator.edge_occupancy.get_mut(&edge) {
        let stub_cells = ((removal.stub_m / cell_m).floor() as usize).min(cells.cells_long);
        cells.left.drain(..stub_cells);
        cells.right.drain(..stub_cells);
        cells.cells_long -= stub_cells;
    }
    if let Some(journal) = journal {
        journal.push(RoadSplitDependentsUndo::Stub {
            edge,
            buildings,
            occupancy,
        });
    }
}
