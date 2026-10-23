// SPDX-License-Identifier: GPL-2.0-only

//! Span pieces: the existing section model between an edge's two mouth stations.
//!
//! The product rules are the legacy section model, unchanged: stations at the physical knots,
//! class steps of 8 / 12 / 10 m, a 2 m cadence through profile transitions and width tapers,
//! protected samples, the width-change taper at degree-two width-change nodes, a section frame
//! from centered secants with exact mouth directions, the junction profile-plane blend, and the
//! bisected shared axis at pass-through nodes. A span runs between the mouth sections its two
//! node pieces evaluated; an end without a node piece (a pass-through node) evaluates its own at
//! the graph clip.

mod model;

use model::SpanModel;
pub(crate) use model::SpanScratch;
pub(super) use model::{
    PASS_THROUGH_DOT_THRESHOLD, edge_points, incident_direction, is_surface_edge,
};

use super::index::{PieceFootprint, RenderGrid};
use super::layout::EdgeEnd;
use super::mesh::{PieceMesh, build_span_mesh};
use super::profile::RoadProfile;
use super::{RoadSection, RoadVec3};
use crate::simulation::network::graph::RegionGraph;
use std::sync::Arc;

/// Cross-section at one end of a span: evaluated once, by the node piece at that end (or by the
/// span model at a pass-through node), and copied into every piece that meets there, so their
/// shared vertices are bit-identical.
#[derive(Clone, Debug, PartialEq)]
pub struct MouthSection {
    /// The evaluated section at the mouth station.
    pub section: RoadSection,
    /// Profile the section was evaluated with; its levels place the curb-face vertex pairs.
    pub profile: RoadProfile,
    /// Section vertices left to right: one per band boundary per level, so a ground / raised
    /// boundary has two vertices with equal XZ, the left band's first.
    pub vertices: Box<[RoadVec3]>,
}

/// Immutable span piece of one edge: its sections between the two mouths and their mesh.
#[derive(Debug, PartialEq)]
pub struct SpanPiece {
    edge_idx: usize,
    // The edge's start and end nodes when built, so a reconnected edge's old nodes rebuild.
    nodes: [u32; 2],
    sections: Box<[RoadSection]>,
    start_mouth: Arc<MouthSection>,
    end_mouth: Arc<MouthSection>,
    mesh: Arc<PieceMesh>,
    footprint: PieceFootprint,
}

impl SpanPiece {
    /// Owning edge id.
    pub fn edge_idx(&self) -> usize {
        self.edge_idx
    }

    /// Sections from the start mouth to the end mouth, in increasing station order.
    pub fn sections(&self) -> &[RoadSection] {
        &self.sections
    }

    /// Mouth section at the edge's start node.
    pub fn start_mouth(&self) -> &MouthSection {
        &self.start_mouth
    }

    /// Mouth section at the edge's end node.
    pub fn end_mouth(&self) -> &MouthSection {
        &self.end_mouth
    }

    /// The piece's mesh.
    pub fn mesh(&self) -> &PieceMesh {
        &self.mesh
    }

    pub(super) fn shared_mesh(&self) -> &Arc<PieceMesh> {
        &self.mesh
    }

    pub(super) fn footprint(&self) -> &PieceFootprint {
        &self.footprint
    }

    pub(super) fn nodes(&self) -> [u32; 2] {
        self.nodes
    }
}

/// Builds the span piece of `edge_idx` between the node pieces' mouth sections at its start and
/// end, or `None` when the edge has no span: it is missing, deleted, not a road or footpath, or
/// its two mouths leave no length between them.
///
/// `O(sections * bands)` plus the piece grid; allocates only the piece.
pub(crate) fn build_span_piece(
    graph: &RegionGraph,
    edge_idx: usize,
    mouths: [Option<&Arc<MouthSection>>; 2],
    grid: RenderGrid,
    scratch: &mut SpanScratch,
) -> Option<SpanPiece> {
    let SpanScratch {
        points,
        cumulative,
        stations,
        protected,
        footprint,
    } = scratch;
    let model = SpanModel::new(
        graph,
        edge_idx,
        points,
        cumulative,
        mouths.map(|mouth| mouth.map(Arc::as_ref)),
    )?;
    let own_mouth = |end: EdgeEnd| Arc::new(model.mouth_section(end));
    let start_mouth = mouths[0].map_or_else(|| own_mouth(EdgeEnd::Start), Arc::clone);
    let end_mouth = mouths[1].map_or_else(|| own_mouth(EdgeEnd::End), Arc::clone);
    model.interior_stations(stations, protected);
    let mut sections = Vec::with_capacity(stations.len() + 2);
    sections.push(start_mouth.section.clone());
    sections.extend(stations.iter().map(|&s_m| model.section_at(s_m)));
    sections.push(end_mouth.section.clone());
    let mesh = build_span_mesh(&sections, &start_mouth, &end_mouth, model.layout());
    let footprint = PieceFootprint::of_span(&mesh, model.layout().stride(), grid, footprint);
    let edge = graph.edge(edge_idx);
    Some(SpanPiece {
        edge_idx,
        nodes: [edge.start_node, edge.end_node].map(|node| graph.get_valid_node(node)),
        sections: sections.into_boxed_slice(),
        start_mouth,
        end_mouth,
        mesh: Arc::new(mesh),
        footprint,
    })
}

#[cfg(test)]
mod tests;
