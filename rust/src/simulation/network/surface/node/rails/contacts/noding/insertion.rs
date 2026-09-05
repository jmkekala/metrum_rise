// SPDX-License-Identifier: GPL-2.0-only

//! Contact noding insertion into generated contours and source constraints.

use super::super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeRailConstraint,
    NodeRailGenerationError, NodeRailPointKey, RoadVec3, generated_contour_keys,
    generated_point_key_lies_on_segment, generated_segment_parameter_key,
    height_for_key_on_generated_edge, road_point_from_key, road_point_key,
    set_generated_contour_from_keys,
};
use super::{ContactEdgeInsertions, ContactInsertionsByIndex, ContactNodingCandidate};
use std::collections::BTreeSet;

pub(super) fn insert_contact_noding_candidates(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    candidates: &[ContactNodingCandidate],
) -> Result<bool, NodeRailGenerationError> {
    let mut insertions_by_contour = ContactInsertionsByIndex::new();
    for &(contour_index, edge, insert_key) in candidates {
        insertions_by_contour
            .entry(contour_index)
            .or_default()
            .entry(edge)
            .or_default()
            .insert(insert_key);
    }

    let mut inserted_any = false;
    for (contour_index, insertions_by_edge) in insertions_by_contour {
        inserted_any |= insert_keys_on_generated_contour_edges(
            contours,
            constraints,
            contour_index,
            insertions_by_edge,
        )?;
    }
    Ok(inserted_any)
}

fn insert_keys_on_generated_contour_edges(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    contour_index: usize,
    insertions_by_edge: ContactEdgeInsertions,
) -> Result<bool, NodeRailGenerationError> {
    let Some(contour) = contours.get_mut(contour_index) else {
        return Ok(false);
    };
    let keys = generated_contour_keys(contour);
    if keys.len() < 2 {
        return Ok(false);
    }

    let height_points = contour.height_points_world.clone();
    let mut new_keys = Vec::with_capacity(keys.len());
    let mut new_height_points = height_points
        .as_ref()
        .filter(|points| points.len() == keys.len())
        .map(|_| Vec::with_capacity(keys.len()));
    let mut inserted_any = false;

    for index in 0..keys.len() {
        let next = (index + 1) % keys.len();
        let start = keys[index];
        let end = keys[next];
        new_keys.push(start);
        if let (Some(height_points), Some(new_height_points)) =
            (height_points.as_ref(), new_height_points.as_mut())
        {
            new_height_points.push(height_points[index]);
        }

        let edge = GeneratedContourDirectedEdge { start, end };
        let Some(insertions) = insertions_by_edge.get(&edge) else {
            continue;
        };
        let mut insertions = sorted_edge_insertions(insertions, start, end);
        for insert_key in insertions.drain(..) {
            inserted_any = true;
            new_keys.push(insert_key);
            if let (Some(height_points), Some(new_height_points)) =
                (height_points.as_ref(), new_height_points.as_mut())
            {
                let Some(height_m) = height_for_key_on_generated_edge(
                    insert_key,
                    start,
                    end,
                    height_points[index].y,
                    height_points[next].y,
                ) else {
                    return Err(NodeRailGenerationError::InvalidHeightCarrier {
                        kind: contour.kind,
                        mouth_order_index: contour.source_mouth_order_index,
                        band_index: contour.source_band_index,
                        reason: "contact_noding_height_not_on_source_edge",
                    });
                };
                let point = road_point_from_key(insert_key);
                new_height_points.push(RoadVec3::new(point.x, height_m, point.y));
            }
        }
    }

    if !inserted_any {
        return Ok(false);
    }
    remove_generated_contour_spikes_with_height_points(&mut new_keys, new_height_points.as_mut());
    if new_keys == keys {
        return Ok(false);
    }
    contour.height_points_world = new_height_points;
    set_generated_contour_from_keys(contour, constraints, new_keys)?;
    Ok(generated_contour_keys(contour) != keys)
}

pub(super) fn insert_keys_on_generated_source_constraints(
    constraints: &mut [NodeRailConstraint],
    insertions_by_constraint: ContactInsertionsByIndex,
) -> bool {
    let mut inserted_any = false;
    for constraint in constraints {
        let Some(insertions_by_edge) = insertions_by_constraint.get(&constraint.constraint_index)
        else {
            continue;
        };
        let keys = constraint
            .points_xz
            .iter()
            .copied()
            .map(road_point_key)
            .collect::<Vec<_>>();
        if keys.len() < 2 {
            continue;
        }
        let mut new_keys = Vec::with_capacity(keys.len());
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            new_keys.push(start);
            let edge = GeneratedContourDirectedEdge { start, end };
            let Some(insertions) = insertions_by_edge.get(&edge) else {
                continue;
            };
            let insertions = sorted_edge_insertions(insertions, start, end);
            if !insertions.is_empty() {
                inserted_any = true;
            }
            new_keys.extend(insertions);
        }
        if let Some(last) = keys.last().copied() {
            new_keys.push(last);
        }
        new_keys.dedup();
        if new_keys != keys {
            constraint.points_xz = new_keys.into_iter().map(road_point_from_key).collect();
        }
    }
    inserted_any
}

fn sorted_edge_insertions(
    insertions: &BTreeSet<NodeRailPointKey>,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> Vec<NodeRailPointKey> {
    let mut insertions = insertions
        .iter()
        .copied()
        .filter(|point| *point != start && *point != end)
        .filter(|point| generated_point_key_lies_on_segment(*point, start, end))
        .collect::<Vec<_>>();
    insertions.sort_by_key(|point| generated_segment_parameter_key(start, end, *point));
    insertions.dedup();
    insertions
}

fn remove_generated_contour_spikes_with_height_points(
    keys: &mut Vec<NodeRailPointKey>,
    mut height_points: Option<&mut Vec<RoadVec3>>,
) {
    let mut index = 1;
    while index < keys.len() {
        if keys[index - 1] == keys[index] {
            keys.remove(index);
            if let Some(height_points) = height_points.as_mut() {
                height_points.remove(index);
            }
        } else {
            index += 1;
        }
    }
    loop {
        if keys.len() < 3 {
            return;
        }
        let mut removed = false;
        for index in 0..keys.len() {
            let previous = if index == 0 {
                keys.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == keys.len() {
                0
            } else {
                index + 1
            };
            if keys[previous] == keys[next] {
                keys.remove(index);
                if let Some(height_points) = height_points.as_mut() {
                    height_points.remove(index);
                }
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_removal_preserves_matching_height_points() {
        let mut keys = vec![(0, 0), (1, 0), (0, 0), (0, 2)];
        let mut heights = vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 1.0, 0.0),
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(0.0, 2.0, 2.0),
        ];

        remove_generated_contour_spikes_with_height_points(&mut keys, Some(&mut heights));

        assert_eq!(keys, vec![(0, 0), (0, 0)]);
        assert_eq!(heights.len(), keys.len());
        assert_eq!(heights[1].x, 0.0);
        assert_eq!(heights[1].y, 0.0);
    }
}
