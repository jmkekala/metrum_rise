// SPDX-License-Identifier: GPL-2.0-only

//! Earthwork visibility policy and span section range selection.

use super::super::{RoadSurfaceSection, RoadSurfaceSystem};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;

const BRIDGE_ABUTMENT_CONTACT_CLEARANCE_M: f32 = 1.0;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn node_piece_uses_earthworks(
        &self,
        graph: &RegionGraph,
        node_id: u32,
        terrain: &TerrainSystem,
    ) -> bool {
        if node_id as usize >= graph.node_adjacency_count() {
            return false;
        }

        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || !Self::is_surface_edge(edge) {
                continue;
            }
            if edge.class == EdgeClass::Standard {
                return true;
            }

            if edge.class == EdgeClass::Bridge {
                continue;
            }

            let at_start = graph.get_valid_node(edge.start_node) == node_id;
            if self.tunnel_throat_is_visible(edge_idx, at_start, terrain) {
                return true;
            }
        }

        false
    }

    pub(in crate::simulation::network::surface) fn earthwork_section_ranges_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        let Some((start_index, end_index)) =
            self.corridor_index_range_for_edge(graph, edge_idx, edge, sections)
        else {
            return Vec::new();
        };

        match edge.class {
            EdgeClass::Standard => vec![(start_index, end_index)],
            EdgeClass::Bridge => self.bridge_endpoint_abutment_section_ranges(
                sections,
                start_index,
                end_index,
                terrain,
            ),
            EdgeClass::Tunnel => {
                self.tunnel_visible_section_ranges(sections, start_index, end_index, terrain)
            }
        }
    }

    fn bridge_endpoint_abutment_section_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        start_index: usize,
        end_index: usize,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        if end_index <= start_index {
            return Vec::new();
        }

        let mut ranges = Vec::new();
        if self.bridge_section_contacts_terrain(&sections[start_index], terrain) {
            let mut contact_end = start_index;
            while contact_end < end_index
                && self.bridge_section_contacts_terrain(&sections[contact_end + 1], terrain)
            {
                contact_end += 1;
            }
            let transition_end = (contact_end + 1).min(end_index);
            if transition_end > start_index {
                ranges.push((start_index, transition_end));
            }
        }

        if self.bridge_section_contacts_terrain(&sections[end_index], terrain) {
            let mut contact_start = end_index;
            while contact_start > start_index
                && self.bridge_section_contacts_terrain(&sections[contact_start - 1], terrain)
            {
                contact_start -= 1;
            }
            let transition_start = contact_start.saturating_sub(1).max(start_index);
            if end_index > transition_start {
                if let Some(last) = ranges.last_mut() {
                    if transition_start <= last.1 {
                        last.1 = end_index;
                    } else {
                        ranges.push((transition_start, end_index));
                    }
                } else {
                    ranges.push((transition_start, end_index));
                }
            }
        }

        ranges
    }

    /// Returns whether any paved boundary of a bridge section reaches its terrain contact zone.
    pub(in crate::simulation::network::surface) fn bridge_section_contacts_terrain(
        &self,
        section: &RoadSurfaceSection,
        terrain: &TerrainSystem,
    ) -> bool {
        section.bands.iter().any(|band| {
            [
                (band.lateral_start_m, band.height_start_m),
                (band.lateral_end_m, band.height_end_m),
            ]
            .into_iter()
            .any(|(lateral_m, height_m)| {
                let point = Self::section_boundary_world_point_static(section, lateral_m, height_m);
                let terrain_height_m = terrain.sample_height_world(point.x as f32, point.z as f32)
                    * crate::config::HEIGHT_SCALE;
                point.y as f32 - terrain_height_m <= BRIDGE_ABUTMENT_CONTACT_CLEARANCE_M
            })
        })
    }

    fn corridor_index_range_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        edge: &Edge,
        sections: &[RoadSurfaceSection],
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 || edge_idx >= graph.edge_count() {
            return None;
        }

        // Tunnel portals are structural endpoint regions; trimming them by the ordinary road-width
        // handoff can erase portals or collapse short spans into one full-length stamp. Bridge
        // abutments do use the visible handoff because the adjacent node owns the remaining cutout.
        if edge.class == EdgeClass::Tunnel {
            return Some((0, sections.len().saturating_sub(1)));
        }

        let total_length = sections.last()?.s_m.max(0.0);
        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let (start_handoff, end_handoff) = self
            .visual_edge_mouth_policy_for_edge(
                graph,
                edge_idx,
                edge,
                total_length,
                start_kind,
                end_kind,
                false,
                false,
            )
            .ownership_range?;
        Self::section_index_range_for_s_bounds(sections, start_handoff, end_handoff)
    }

    pub(in crate::simulation::network::surface) fn tunnel_visible_section_ranges(
        &self,
        sections: &[RoadSurfaceSection],
        start_index: usize,
        end_index: usize,
        terrain: &TerrainSystem,
    ) -> Vec<(usize, usize)> {
        if end_index <= start_index {
            return Vec::new();
        }

        let mut ranges = Vec::new();

        if self.section_is_tunnel_surface_visible(&sections[start_index], terrain) {
            let mut visible_end = start_index;
            while visible_end < end_index
                && self.section_is_tunnel_surface_visible(&sections[visible_end + 1], terrain)
            {
                visible_end += 1;
            }
            let transition_end = (visible_end + 1).min(end_index);
            if transition_end > start_index {
                ranges.push((start_index, transition_end));
            }
        }

        if self.section_is_tunnel_surface_visible(&sections[end_index], terrain) {
            let mut visible_start = end_index;
            while visible_start > start_index
                && self.section_is_tunnel_surface_visible(&sections[visible_start - 1], terrain)
            {
                visible_start -= 1;
            }
            let transition_start = visible_start.saturating_sub(1).max(start_index);
            if end_index > transition_start {
                if let Some(last) = ranges.last_mut() {
                    if transition_start <= last.1 {
                        last.1 = end_index;
                    } else {
                        ranges.push((transition_start, end_index));
                    }
                } else {
                    ranges.push((transition_start, end_index));
                }
            }
        }

        ranges
    }
}
