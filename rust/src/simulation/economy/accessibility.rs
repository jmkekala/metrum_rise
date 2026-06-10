//! Economy-side reachability helpers for candidate planning.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

/// Sentinel component id for nodes or buildings with no usable access.
pub(crate) const NO_COMPONENT: u32 = u32::MAX;

/// Connected-component labels for one transit mode.
#[derive(Clone, Debug)]
pub(crate) struct ModeComponentIndex {
    component_by_node: Vec<u32>,
}

/// Up to two component labels reachable from a building entrance for one mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BuildingModeComponents {
    components: [u32; 2],
    count: u8,
}

impl Default for BuildingModeComponents {
    fn default() -> Self {
        Self {
            components: [NO_COMPONENT; 2],
            count: 0,
        }
    }
}

impl BuildingModeComponents {
    /// Returns the component labels in deterministic order.
    pub(crate) fn as_slice(&self) -> &[u32] {
        &self.components[..self.count as usize]
    }

    /// Returns the fixed backing array and active component count for allocation-free iteration.
    pub(crate) fn raw_parts(self) -> ([u32; 2], usize) {
        (self.components, self.count as usize)
    }

    fn push(&mut self, component: u32) {
        if component == NO_COMPONENT || self.as_slice().contains(&component) {
            return;
        }
        let idx = self.count as usize;
        if idx < self.components.len() {
            self.components[idx] = component;
            self.count += 1;
        }
    }
}

impl ModeComponentIndex {
    /// Builds undirected reachability labels for the requested transit mode.
    pub(crate) fn build(graph: &RegionGraph, transit_flag: u8) -> Self {
        let node_count = graph.node_count();
        let mut parent: Vec<usize> = (0..node_count).collect();
        let mut active = vec![false; node_count];

        for edge in graph.edges() {
            if edge.deleted || (edge.allowed_types & transit_flag) == 0 {
                continue;
            }
            let start = graph.get_valid_node(edge.start_node) as usize;
            let end = graph.get_valid_node(edge.end_node) as usize;
            if start >= node_count || end >= node_count {
                continue;
            }
            active[start] = true;
            active[end] = true;
            unite(start, end, &mut parent);
        }

        let mut root_to_component = BTreeMap::new();
        let mut component_by_node = vec![NO_COMPONENT; node_count];
        for node in 0..node_count {
            if !active[node] {
                continue;
            }
            let root = find(node, &mut parent);
            let next_id = root_to_component.len() as u32;
            let component = *root_to_component.entry(root).or_insert(next_id);
            component_by_node[node] = component;
        }

        Self { component_by_node }
    }

    /// Returns the component label for a graph node, or [`NO_COMPONENT`].
    pub(crate) fn node_component(&self, node: u32) -> u32 {
        self.component_by_node
            .get(node as usize)
            .copied()
            .unwrap_or(NO_COMPONENT)
    }

    /// Returns the components reachable from a building's entrance for this mode.
    pub(crate) fn building_components(
        &self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        building_idx: usize,
        transit_flag: u8,
    ) -> BuildingModeComponents {
        if building_idx >= allocator.buildings.len() || building_idx >= allocator.entrances.len() {
            return BuildingModeComponents::default();
        }
        let entrance = &allocator.entrances[building_idx];
        if !entrance_supports_mode(entrance, transit_flag) {
            return BuildingModeComponents::default();
        }
        let Some(edge) = graph.get_edge(entrance.edge_idx) else {
            return BuildingModeComponents::default();
        };
        if edge.deleted || (edge.allowed_types & transit_flag) == 0 {
            return BuildingModeComponents::default();
        }

        let mut components = BuildingModeComponents::default();
        components.push(self.node_component(graph.get_valid_node(edge.start_node)));
        components.push(self.node_component(graph.get_valid_node(edge.end_node)));
        components
    }
}

fn entrance_supports_mode(
    entrance: &crate::simulation::buildings::allocator::BuildingEntrance,
    transit_flag: u8,
) -> bool {
    if transit_flag == TransitFlags::CAR {
        entrance.car_lane_fwd != usize::MAX || entrance.car_lane_bkw != usize::MAX
    } else if transit_flag == TransitFlags::FOOT {
        entrance.foot_lane_fwd != usize::MAX || entrance.foot_lane_bkw != usize::MAX
    } else {
        false
    }
}

fn find(mut i: usize, parent: &mut [usize]) -> usize {
    let mut root = i;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[i] != i {
        let next = parent[i];
        parent[i] = root;
        i = next;
    }
    root
}

fn unite(i: usize, j: usize, parent: &mut [usize]) {
    let root_i = find(i, parent);
    let root_j = find(j, parent);
    if root_i != root_j {
        parent[root_i] = root_j;
    }
}
