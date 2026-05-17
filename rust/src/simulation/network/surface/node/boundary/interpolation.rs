//! Authorized sub-budget footprint boundary height interpolation.

use super::*;

impl NodeFootprintBoundaryExportSources {
    pub(in crate::simulation::network::surface) fn interpolate_missing_authorized_footprint_boundary_heights(
        &mut self,
        vertices: &mut [(arrangement::NodeArrangementKey, Option<i64>)],
    ) -> Result<(), NodeBoundaryExportError> {
        let Some(_first_missing_key) = vertices
            .iter()
            .find_map(|(key, height_mm)| height_mm.is_none().then_some(*key))
        else {
            return Ok(());
        };
        let Some(first_solved_index) = vertices
            .iter()
            .position(|(_, height_mm)| height_mm.is_some())
        else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };
        if vertices
            .iter()
            .filter(|(_, height_mm)| height_mm.is_some())
            .count()
            < 2
        {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        }

        let mut ordered_indices = Vec::with_capacity(vertices.len() + 1);
        ordered_indices.extend(first_solved_index..vertices.len());
        ordered_indices.extend(0..=first_solved_index);

        let mut start_pos = 0;
        while start_pos + 1 < ordered_indices.len() {
            let start_index = ordered_indices[start_pos];
            let Some(start_height_mm) = vertices[start_index].1 else {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            };
            let Some(end_pos) = (start_pos + 1..ordered_indices.len())
                .find(|pos| vertices[ordered_indices[*pos]].1.is_some())
            else {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            };
            if end_pos == start_pos + 1 {
                start_pos = end_pos;
                continue;
            }

            let end_index = ordered_indices[end_pos];
            let Some(end_height_mm) = vertices[end_index].1 else {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            };
            let Some(start_candidate) =
                self.height_candidate_at_point(vertices[start_index].0, start_height_mm)?
            else {
                return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
            };
            let Some(end_candidate) =
                self.height_candidate_at_point(vertices[end_index].0, end_height_mm)?
            else {
                return Err(NodeBoundaryExportError::MissingEarthworkBoundarySource);
            };
            if !self.missing_footprint_boundary_run_is_authorized_subbudget(
                vertices,
                &ordered_indices[start_pos..=end_pos],
            )? {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            }

            let mut cumulative_lengths = Vec::with_capacity(end_pos - start_pos + 1);
            cumulative_lengths.push(0.0);
            let mut total_length_m = 0.0;
            for pair_pos in start_pos..end_pos {
                total_length_m += arrangement_key_distance_m(
                    vertices[ordered_indices[pair_pos]].0,
                    vertices[ordered_indices[pair_pos + 1]].0,
                );
                cumulative_lengths.push(total_length_m);
            }
            if total_length_m <= f64::EPSILON {
                return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
            }

            for run_offset in 1..cumulative_lengths.len() - 1 {
                let index = ordered_indices[start_pos + run_offset];
                let t = cumulative_lengths[run_offset] / total_length_m;
                let height_mm = (start_height_mm as f64
                    + (end_height_mm - start_height_mm) as f64 * t)
                    .round() as i64;
                vertices[index].1 = Some(height_mm);
                self.insert_contour_interpolated_boundary_source(
                    vertices[index].0,
                    height_mm,
                    start_candidate,
                    end_candidate,
                );
            }
            start_pos = end_pos;
        }
        Ok(())
    }

    fn missing_footprint_boundary_run_is_authorized_subbudget(
        &self,
        vertices: &[(arrangement::NodeArrangementKey, Option<i64>)],
        ordered_indices: &[usize],
    ) -> Result<bool, NodeBoundaryExportError> {
        if !footprint_boundary_missing_run_is_subbudget(vertices, ordered_indices) {
            return Ok(false);
        }
        let Some(start_index) = ordered_indices.first().copied() else {
            return Ok(false);
        };
        let Some(end_index) = ordered_indices.last().copied() else {
            return Ok(false);
        };
        let Some(start_height_mm) = vertices[start_index].1 else {
            return Ok(false);
        };
        let Some(end_height_mm) = vertices[end_index].1 else {
            return Ok(false);
        };
        if !self.has_exact_final_owned_footprint_boundary_support_at_point(
            arrangement_boundary_point_key_with_height(vertices[start_index].0, start_height_mm),
        ) || !self.has_exact_final_owned_footprint_boundary_support_at_point(
            arrangement_boundary_point_key_with_height(vertices[end_index].0, end_height_mm),
        ) {
            return Ok(false);
        }

        for missing_index in ordered_indices
            .iter()
            .copied()
            .skip(1)
            .take(ordered_indices.len().saturating_sub(2))
        {
            if vertices[missing_index].1.is_some() {
                return Ok(false);
            }
            if self
                .height_candidate_at_boundary_vertex(vertices[missing_index].0)?
                .is_some()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn insert_contour_interpolated_boundary_source(
        &mut self,
        key: arrangement::NodeArrangementKey,
        height_mm: i64,
        start: NodeFootprintBoundaryHeightCandidate,
        end: NodeFootprintBoundaryHeightCandidate,
    ) {
        let owner = if node_footprint_direct_vertex_ordering(start.source, end.source).is_ge() {
            start.source
        } else {
            end.source
        };
        let candidate = NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: node_footprint_boundary_source_start_direct_source(
                    start.source.source,
                ),
                owning_segment_end: node_footprint_boundary_source_end_direct_source(
                    end.source.source,
                ),
                height_mm,
            },
            owner_kind: owner.owner_kind,
            owner_index: owner.owner_index,
        };
        self.insert_boundary_vertex_source(key, height_mm, candidate);
    }
}

fn node_footprint_boundary_source_start_direct_source(
    source: NodeFootprintBoundaryVertexSource,
) -> NodeFootprintBoundaryDirectSource {
    match source {
        NodeFootprintBoundaryVertexSource::Direct(direct) => direct,
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start,
            ..
        } => owning_segment_start,
    }
}

fn node_footprint_boundary_source_end_direct_source(
    source: NodeFootprintBoundaryVertexSource,
) -> NodeFootprintBoundaryDirectSource {
    match source {
        NodeFootprintBoundaryVertexSource::Direct(direct) => direct,
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_end, ..
        } => owning_segment_end,
    }
}

fn arrangement_boundary_point_key_with_height(
    key: arrangement::NodeArrangementKey,
    height_mm: i64,
) -> ArrangementBoundaryPointKey {
    ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: height_mm,
    }
}

fn footprint_boundary_missing_run_is_subbudget(
    vertices: &[(arrangement::NodeArrangementKey, Option<i64>)],
    ordered_indices: &[usize],
) -> bool {
    ordered_indices.windows(3).all(|local_indices| {
        let points = [
            arrangement_key_flat_boundary_point(vertices[local_indices[0]].0),
            arrangement_key_flat_boundary_point(vertices[local_indices[1]].0),
            arrangement_key_flat_boundary_point(vertices[local_indices[2]].0),
        ];
        RoadSurfaceSystem::signed_polygon_area_xz(&points).abs()
            <= boundary_points_numeric_area_budget_m2(&points)
    })
}

fn arrangement_key_flat_boundary_point(key: arrangement::NodeArrangementKey) -> Vector3 {
    Vector3::new(
        (key.x_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE) as f32,
        0.0,
        (key.z_key() as f64 / ROAD_OVERLAY_COORDINATE_SCALE) as f32,
    )
}
