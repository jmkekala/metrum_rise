//! Height-owned arrangement region construction.

use super::super::super::{NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M};
use super::super::height::{NodeGradeCarrierDecision, NodeGradeVertexAuthority};
use super::super::height::{NodeHeightedRegion, NodeHeightedVertex};
use super::super::keys::{
    SURFACE_MM_PER_M, SURFACE_XZ_KEY_SCALE, SurfaceSegmentParameter, SurfaceXzKey,
};
use super::super::segments::interpolate_height_i64;
use super::build::{node_grade_decision_rank, quantize_height_m};
use super::edges::{PendingArrangementEdge, loop_edges};
use super::seams::{NodeRegionSeamConstraint, NodeSeamSource};
use super::{
    NodeArrangement, NodeArrangementEdgeId, NodeArrangementError, NodeArrangementKey,
    NodeArrangementVertex, NodeArrangementVertexId, NodeBandHeightFieldId, NodeBandOwner,
    NodeOwnedRegion, NodeOwnedRegionId,
};
use std::collections::BTreeSet;

#[derive(Clone)]
pub(super) struct PendingArrangementRegion {
    pub(super) region_index: usize,
    pub(super) owner: NodeBandOwner,
    pub(super) height_field_id: NodeBandHeightFieldId,
    pub(super) outer_loop: Vec<NodeArrangementVertexId>,
    pub(super) holes: Vec<Vec<NodeArrangementVertexId>>,
    pub(super) area_m2: f32,
    pub(super) seam_constraints: Vec<NodeRegionSeamConstraint>,
}

impl PendingArrangementRegion {
    pub(super) fn edge_count(&self) -> usize {
        self.outer_loop.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    pub(super) fn loop_edges(
        &self,
        vertices: &[NodeArrangementVertex],
    ) -> Vec<PendingArrangementEdge> {
        let mut edges = loop_edges(&self.outer_loop, vertices);
        for hole in &self.holes {
            edges.extend(loop_edges(hole, vertices));
        }
        edges
    }
}

impl NodeArrangement {
    pub(super) fn node_pending_region_edges(
        &mut self,
        pending_regions: &mut [PendingArrangementRegion],
    ) -> Result<(), NodeArrangementError> {
        let split_keys = self
            .vertices
            .iter()
            .map(NodeArrangementVertex::key)
            .collect::<BTreeSet<_>>();
        for pending in pending_regions {
            self.node_pending_loop_edges(
                pending.owner,
                pending.height_field_id,
                &mut pending.outer_loop,
                &split_keys,
            )?;
            for hole in &mut pending.holes {
                self.node_pending_loop_edges(
                    pending.owner,
                    pending.height_field_id,
                    hole,
                    &split_keys,
                )?;
            }
        }
        Ok(())
    }

    fn node_pending_loop_edges(
        &mut self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        loop_vertices: &mut Vec<NodeArrangementVertexId>,
        split_keys: &BTreeSet<NodeArrangementKey>,
    ) -> Result<(), NodeArrangementError> {
        if loop_vertices.len() < 3 {
            return Ok(());
        }
        let original = loop_vertices.clone();
        let mut noded = Vec::with_capacity(original.len());
        for index in 0..original.len() {
            let start_id = original[index];
            let end_id = original[(index + 1) % original.len()];
            noded.push(start_id);
            let (Some(start), Some(end)) = (
                self.vertices.get(start_id.index()).cloned(),
                self.vertices.get(end_id.index()).cloned(),
            ) else {
                continue;
            };
            for (split_key, parameter) in
                interior_split_keys_on_edge(start.key(), end.key(), split_keys)
            {
                let split_id = self.insert_split_vertex_on_arrangement_edge(
                    owner,
                    height_field_id,
                    &start,
                    &end,
                    split_key,
                    parameter,
                )?;
                if noded.last().copied() != Some(split_id) {
                    noded.push(split_id);
                }
            }
        }
        clean_arrangement_loop_vertices(&mut noded, &self.vertices);
        *loop_vertices = noded;
        Ok(())
    }

    fn insert_split_vertex_on_arrangement_edge(
        &mut self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        start: &NodeArrangementVertex,
        end: &NodeArrangementVertex,
        split_key: NodeArrangementKey,
        parameter: SurfaceSegmentParameter,
    ) -> Result<NodeArrangementVertexId, NodeArrangementError> {
        let split_surface_key = split_key.surface_key();
        let point_xz = split_surface_key.to_road_xz();
        let (height_m, grade_authority) = self
            .existing_owner_vertex_at_split_key(split_key, owner, height_field_id)
            .unwrap_or_else(|| {
                let height_mm =
                    interpolate_height_i64(start.height_mm(), end.height_mm(), parameter);
                let height_m = height_mm as f64 / SURFACE_MM_PER_M;
                (
                    height_m,
                    NodeGradeVertexAuthority::new(
                        point_xz,
                        height_m,
                        owner,
                        height_field_id,
                        NodeGradeCarrierDecision::SameOwnerCanonicalVertex,
                    ),
                )
            });
        self.insert_vertex_with_grade_authority(
            point_xz,
            height_m,
            [owner],
            height_field_id,
            [NodeSeamSource::for_owner(owner)],
            grade_authority,
        )
    }

    fn existing_owner_vertex_at_split_key(
        &self,
        split_key: NodeArrangementKey,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
    ) -> Option<(f64, NodeGradeVertexAuthority)> {
        let candidates = self
            .vertices
            .iter()
            .filter(|vertex| {
                vertex.key() == split_key
                    && vertex.height_field_id() == height_field_id
                    && vertex.owners().contains(&owner)
            })
            .collect::<Vec<_>>();
        let first = candidates.first()?;
        if candidates
            .iter()
            .any(|candidate| candidate.height_mm() != first.height_mm())
        {
            return None;
        }
        let grade_authority =
            candidates
                .iter()
                .skip(1)
                .fold(first.grade_authority(), |authority, candidate| {
                    merged_split_vertex_grade_authority(authority, candidate.grade_authority())
                });
        Some((first.height_m(), grade_authority))
    }

    pub(crate) fn push_region(
        &mut self,
        owner: NodeBandOwner,
        height_field_id: NodeBandHeightFieldId,
        outer_loop: Vec<NodeArrangementVertexId>,
        holes: Vec<Vec<NodeArrangementVertexId>>,
        boundary_edges: Vec<NodeArrangementEdgeId>,
        area_m2: f32,
        seam_constraints: Vec<NodeRegionSeamConstraint>,
    ) -> NodeOwnedRegionId {
        let id = NodeOwnedRegionId(self.regions.len());
        self.regions.push(NodeOwnedRegion {
            id,
            owner,
            height_field_id,
            outer_loop,
            holes,
            boundary_edges,
            area_m2,
            seam_constraints,
        });
        id
    }

    pub(super) fn pending_region(
        &mut self,
        region_index: usize,
        region: &NodeHeightedRegion,
    ) -> Result<PendingArrangementRegion, NodeArrangementError> {
        if region.shape.is_empty() {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index: 0,
            });
        }

        let mut contours = Vec::with_capacity(region.shape.len());
        for (contour_index, contour) in region.shape.iter().enumerate() {
            contours.push(self.insert_heighted_contour(
                region_index,
                contour_index,
                region,
                contour,
            )?);
        }
        let mut contours = contours.into_iter();
        let outer_loop = contours
            .next()
            .ok_or(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index: 0,
            })?;
        let holes = contours.collect();
        Ok(PendingArrangementRegion {
            region_index,
            owner: region.owner,
            height_field_id: region.height_field_id,
            outer_loop,
            holes,
            area_m2: region.area_m2,
            seam_constraints: region.seam_constraints.clone(),
        })
    }

    fn insert_heighted_contour(
        &mut self,
        region_index: usize,
        contour_index: usize,
        region: &NodeHeightedRegion,
        contour: &[NodeHeightedVertex],
    ) -> Result<Vec<NodeArrangementVertexId>, NodeArrangementError> {
        if contour.len() < 3 {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index,
            });
        }

        let mut loop_vertices = contour
            .iter()
            .map(|vertex| {
                let grade_authority = vertex.grade_authority.ok_or_else(|| {
                    NodeArrangementError::MissingGradeAuthority {
                        region_index,
                        contour_index,
                        key: super::NodeArrangementKey::from_point(vertex.point_xz),
                        owner: region.owner,
                        height_field_id: vertex.height_field_id,
                        height_mm: quantize_height_m(vertex.height_m),
                    }
                })?;
                self.insert_vertex_with_grade_authority(
                    vertex.point_xz,
                    vertex.height_m,
                    [region.owner],
                    vertex.height_field_id,
                    [NodeSeamSource::for_owner(region.owner)],
                    grade_authority,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        clean_arrangement_loop_vertices(&mut loop_vertices, &self.vertices);
        if loop_vertices.len() < 3 {
            return Err(NodeArrangementError::DegenerateRegionContour {
                region_index,
                contour_index,
            });
        }
        Ok(loop_vertices)
    }
}

fn interior_split_keys_on_edge(
    start: NodeArrangementKey,
    end: NodeArrangementKey,
    split_keys: &BTreeSet<NodeArrangementKey>,
) -> Vec<(NodeArrangementKey, SurfaceSegmentParameter)> {
    if start == end {
        return Vec::new();
    }
    let start_surface = start.surface_key();
    let end_surface = end.surface_key();
    let mut keys = split_keys
        .iter()
        .copied()
        .filter(|key| *key != start && *key != end)
        .filter_map(|key| {
            let key = key.surface_key();
            if !key.lies_exactly_on_segment(start_surface, end_surface) {
                return None;
            }
            let parameter = key.exact_line_parameter(start_surface, end_surface)?;
            (parameter > SurfaceSegmentParameter::zero()
                && parameter < SurfaceSegmentParameter::one())
            .then_some((NodeArrangementKey::from_surface_key(key), parameter))
        })
        .collect::<Vec<_>>();
    keys.sort_by_key(|(key, _)| {
        SurfaceXzKey::from_raw_keys(key.x_key(), key.z_key())
            .segment_parameter_key(start_surface, end_surface)
    });
    keys.dedup_by_key(|(key, _)| *key);
    keys
}

fn clean_arrangement_loop_vertices(
    loop_vertices: &mut Vec<NodeArrangementVertexId>,
    vertices: &[NodeArrangementVertex],
) {
    loop {
        let starting_len = loop_vertices.len();
        remove_consecutive_identical_arrangement_vertices(loop_vertices, vertices);
        remove_immediate_backtracking_arrangement_vertices(loop_vertices, vertices);
        if loop_vertices.len() == starting_len {
            break;
        }
        if loop_vertices.len() < 3 {
            break;
        }
    }
}

fn merged_split_vertex_grade_authority(
    existing: NodeGradeVertexAuthority,
    incoming: NodeGradeVertexAuthority,
) -> NodeGradeVertexAuthority {
    if node_grade_decision_rank(incoming.decision) < node_grade_decision_rank(existing.decision) {
        incoming
    } else {
        existing
    }
}

fn remove_consecutive_identical_arrangement_vertices(
    loop_vertices: &mut Vec<NodeArrangementVertexId>,
    vertices: &[NodeArrangementVertex],
) {
    loop_vertices.dedup_by(|left, right| {
        arrangement_vertices_share_exact_key_height_and_field(*left, *right, vertices)
    });
    if loop_vertices.len() >= 2
        && arrangement_vertices_share_exact_key_height_and_field(
            loop_vertices[0],
            *loop_vertices
                .last()
                .expect("arrangement loop has last vertex"),
            vertices,
        )
    {
        loop_vertices.pop();
    }
}

fn remove_immediate_backtracking_arrangement_vertices(
    loop_vertices: &mut Vec<NodeArrangementVertexId>,
    vertices: &[NodeArrangementVertex],
) {
    if loop_vertices.len() < 3 {
        return;
    }
    let Some(index) = (0..loop_vertices.len()).find(|index| {
        let previous = loop_vertices[(*index + loop_vertices.len() - 1) % loop_vertices.len()];
        let current = loop_vertices[*index];
        let next = loop_vertices[(*index + 1) % loop_vertices.len()];
        arrangement_vertices_share_exact_key_height_and_field(previous, next, vertices)
            || arrangement_backtrack_spur_is_numeric_dust(previous, current, next, vertices)
    }) else {
        return;
    };
    loop_vertices.remove(index);
}

fn arrangement_backtrack_spur_is_numeric_dust(
    previous: NodeArrangementVertexId,
    current: NodeArrangementVertexId,
    next: NodeArrangementVertexId,
    vertices: &[NodeArrangementVertex],
) -> bool {
    let (Some(previous), Some(current), Some(next)) = (
        vertices.get(previous.index()),
        vertices.get(current.index()),
        vertices.get(next.index()),
    ) else {
        return false;
    };
    if previous.height_field_id() != next.height_field_id()
        || previous.height_mm() != next.height_mm()
    {
        return false;
    }
    let previous_key = previous.key();
    let next_key = next.key();
    let dust_key_units =
        (f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * SURFACE_XZ_KEY_SCALE).round() as i64;
    let dx = previous_key.x_key() - next_key.x_key();
    let dz = previous_key.z_key() - next_key.z_key();
    if i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz)
        > i128::from(dust_key_units) * i128::from(dust_key_units)
    {
        return false;
    }
    let area_m2 = SurfaceXzKey::raw_tuple_triangle_area_m2_abs(
        (previous_key.x_key(), previous_key.z_key()),
        (current.key().x_key(), current.key().z_key()),
        (next_key.x_key(), next_key.z_key()),
    );
    if area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
        return true;
    }
    area_m2 <= arrangement_backtrack_area_budget_m2(previous, current, next)
}

fn arrangement_backtrack_area_budget_m2(
    previous: &NodeArrangementVertex,
    current: &NodeArrangementVertex,
    next: &NodeArrangementVertex,
) -> f64 {
    let previous_length_m = arrangement_key_distance_m(previous.key(), current.key());
    let next_length_m = arrangement_key_distance_m(current.key(), next.key());
    previous_length_m.max(next_length_m) * f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
}

fn arrangement_key_distance_m(a: NodeArrangementKey, b: NodeArrangementKey) -> f64 {
    let dx = (a.x_key() - b.x_key()) as f64 / SURFACE_XZ_KEY_SCALE;
    let dz = (a.z_key() - b.z_key()) as f64 / SURFACE_XZ_KEY_SCALE;
    dx.hypot(dz)
}

fn arrangement_vertices_share_exact_key_height_and_field(
    left: NodeArrangementVertexId,
    right: NodeArrangementVertexId,
    vertices: &[NodeArrangementVertex],
) -> bool {
    let Some(left) = vertices.get(left.index()) else {
        return false;
    };
    let Some(right) = vertices.get(right.index()) else {
        return false;
    };
    left.key() == right.key()
        && left.height_mm() == right.height_mm()
        && left.height_field_id() == right.height_field_id()
}
