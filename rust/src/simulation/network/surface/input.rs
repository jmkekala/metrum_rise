//! Canonical node-arrangement input extracted from solved road-surface profiles.

#![allow(dead_code)]

use super::arrangement::NodeHeightSource;
use super::backend::{
    RoadVec2, RoadVec3, godot_vec2_to_road, godot_vec3_to_road, godot_vec3_xz_to_road,
};
use super::{
    IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeInputProfileKind {
    Mouth,
    Endpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeInputBoundaryRailRole {
    OuterFootprint {
        adjacent_kind: RoadSurfaceBandKind,
    },
    InteriorBandBoundary {
        left_kind: RoadSurfaceBandKind,
        right_kind: RoadSurfaceBandKind,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeArrangementInput {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) mouths: Vec<NodeInputMouth>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputMouth {
    pub(crate) order_index: usize,
    pub(crate) edge_idx: usize,
    pub(crate) side: IncidentEdgeSide,
    pub(crate) direction_xz: RoadVec2,
    pub(crate) direction_angle_ccw: f64,
    pub(crate) conflict_handoff_distance_m: f64,
    pub(crate) mouth_rails: Vec<NodeInputProfileRail>,
    pub(crate) endpoint_rails: Vec<NodeInputProfileRail>,
    pub(crate) boundary_rails: Vec<NodeInputBoundaryRail>,
    pub(crate) band_intervals: Vec<NodeInputBandInterval>,
    pub(crate) boundary_heights: Vec<NodeInputBoundaryHeight>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputProfileRail {
    pub(crate) profile_kind: NodeInputProfileKind,
    pub(crate) band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) start_world: RoadVec3,
    pub(crate) end_world: RoadVec3,
    pub(crate) height_source: NodeHeightSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputBoundaryRail {
    pub(crate) boundary_index: usize,
    pub(crate) role: NodeInputBoundaryRailRole,
    pub(crate) mouth_world: RoadVec3,
    pub(crate) endpoint_world: RoadVec3,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputBandInterval {
    pub(crate) band_index: usize,
    pub(crate) band_kind: RoadSurfaceBandKind,
    pub(crate) mouth_start_world: RoadVec3,
    pub(crate) mouth_end_world: RoadVec3,
    pub(crate) endpoint_start_world: RoadVec3,
    pub(crate) endpoint_end_world: RoadVec3,
    pub(crate) mouth_height_source: NodeHeightSource,
    pub(crate) endpoint_height_source: NodeHeightSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeInputBoundaryHeight {
    pub(crate) profile_kind: NodeInputProfileKind,
    pub(crate) boundary_index: usize,
    pub(crate) point_xz: RoadVec2,
    pub(crate) height_m: f64,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeInputExtractionError {
    EmptyMouthSet {
        node_id: u32,
    },
    DegenerateDirection {
        edge_idx: usize,
        side: IncidentEdgeSide,
    },
    ProfileBoundaryCountMismatch {
        edge_idx: usize,
        side: IncidentEdgeSide,
        profile_kind: NodeInputProfileKind,
        expected: usize,
        actual: usize,
    },
    EmptyProfileBands {
        edge_idx: usize,
        side: IncidentEdgeSide,
        profile_kind: NodeInputProfileKind,
    },
    ProfileBandCountMismatch {
        edge_idx: usize,
        side: IncidentEdgeSide,
        mouth_band_count: usize,
        endpoint_band_count: usize,
    },
    ProfileBandKindMismatch {
        edge_idx: usize,
        side: IncidentEdgeSide,
        band_index: usize,
        mouth_kind: RoadSurfaceBandKind,
        endpoint_kind: RoadSurfaceBandKind,
    },
    InvalidHandoffDistance {
        edge_idx: usize,
        side: IncidentEdgeSide,
        distance_m: f64,
    },
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_arrangement_input_from_mouths(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Result<NodeArrangementInput, NodeInputExtractionError> {
        NodeArrangementInput::from_ordered_mouths(node_id, piece_kind, mouths)
    }
}

impl NodeArrangementInput {
    pub(crate) fn from_ordered_mouths(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Result<Self, NodeInputExtractionError> {
        if mouths.is_empty() {
            return Err(NodeInputExtractionError::EmptyMouthSet { node_id });
        }

        let mut input_mouths = Vec::with_capacity(mouths.len());
        for (order_index, mouth) in mouths.iter().enumerate() {
            input_mouths.push(NodeInputMouth::from_ordered_mouth(order_index, mouth)?);
        }

        Ok(Self {
            node_id,
            piece_kind,
            mouths: input_mouths,
        })
    }
}

impl NodeInputMouth {
    fn from_ordered_mouth(
        order_index: usize,
        mouth: &OrderedIncidentPieceMouth,
    ) -> Result<Self, NodeInputExtractionError> {
        validate_profile_shape(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Mouth,
            &mouth.profile,
        )?;
        validate_profile_shape(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Endpoint,
            &mouth.endpoint_profile,
        )?;
        validate_profile_pair(mouth)?;

        let direction_xz = normalized_direction(mouth)?;
        let conflict_handoff_distance_m = conflict_handoff_distance_m(mouth, direction_xz)?;
        let mouth_rails = profile_rails(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Mouth,
            &mouth.profile,
        );
        let endpoint_rails = profile_rails(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Endpoint,
            &mouth.endpoint_profile,
        );
        let boundary_rails = boundary_rails(mouth);
        let band_intervals = band_intervals(mouth);
        let mut solved_boundary_heights = boundary_heights(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Mouth,
            &mouth.profile,
        );
        solved_boundary_heights.extend(boundary_heights(
            mouth.edge_idx,
            mouth.side,
            NodeInputProfileKind::Endpoint,
            &mouth.endpoint_profile,
        ));

        Ok(Self {
            order_index,
            edge_idx: mouth.edge_idx,
            side: mouth.side,
            direction_xz,
            direction_angle_ccw: f64::from(mouth.direction_angle_ccw),
            conflict_handoff_distance_m,
            mouth_rails,
            endpoint_rails,
            boundary_rails,
            band_intervals,
            boundary_heights: solved_boundary_heights,
        })
    }
}

fn validate_profile_shape(
    edge_idx: usize,
    side: IncidentEdgeSide,
    profile_kind: NodeInputProfileKind,
    profile: &IncidentMouthProfile,
) -> Result<(), NodeInputExtractionError> {
    if profile.bands.is_empty() {
        return Err(NodeInputExtractionError::EmptyProfileBands {
            edge_idx,
            side,
            profile_kind,
        });
    }

    let expected = profile.bands.len() + 1;
    let actual = profile.boundary_points_world.len();
    if expected != actual {
        return Err(NodeInputExtractionError::ProfileBoundaryCountMismatch {
            edge_idx,
            side,
            profile_kind,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_profile_pair(
    mouth: &OrderedIncidentPieceMouth,
) -> Result<(), NodeInputExtractionError> {
    if mouth.profile.bands.len() != mouth.endpoint_profile.bands.len() {
        return Err(NodeInputExtractionError::ProfileBandCountMismatch {
            edge_idx: mouth.edge_idx,
            side: mouth.side,
            mouth_band_count: mouth.profile.bands.len(),
            endpoint_band_count: mouth.endpoint_profile.bands.len(),
        });
    }

    for (band_index, (mouth_band, endpoint_band)) in mouth
        .profile
        .bands
        .iter()
        .zip(&mouth.endpoint_profile.bands)
        .enumerate()
    {
        if mouth_band.kind != endpoint_band.kind {
            return Err(NodeInputExtractionError::ProfileBandKindMismatch {
                edge_idx: mouth.edge_idx,
                side: mouth.side,
                band_index,
                mouth_kind: mouth_band.kind,
                endpoint_kind: endpoint_band.kind,
            });
        }
    }
    Ok(())
}

fn normalized_direction(
    mouth: &OrderedIncidentPieceMouth,
) -> Result<RoadVec2, NodeInputExtractionError> {
    let direction = godot_vec2_to_road(mouth.direction_xz);
    let length = direction.length();
    if length <= f64::EPSILON {
        return Err(NodeInputExtractionError::DegenerateDirection {
            edge_idx: mouth.edge_idx,
            side: mouth.side,
        });
    }
    Ok(direction / length)
}

fn conflict_handoff_distance_m(
    mouth: &OrderedIncidentPieceMouth,
    direction_xz: RoadVec2,
) -> Result<f64, NodeInputExtractionError> {
    let mut total = 0.0;
    let mut count = 0usize;

    for (mouth_point, endpoint_point) in mouth
        .profile
        .boundary_points_world
        .iter()
        .zip(&mouth.endpoint_profile.boundary_points_world)
    {
        let mouth_xz = godot_vec3_xz_to_road(*mouth_point);
        let endpoint_xz = godot_vec3_xz_to_road(*endpoint_point);
        total += (mouth_xz - endpoint_xz).dot(direction_xz);
        count += 1;
    }

    let distance_m = total / count as f64;
    if !distance_m.is_finite() || distance_m < 0.0 {
        return Err(NodeInputExtractionError::InvalidHandoffDistance {
            edge_idx: mouth.edge_idx,
            side: mouth.side,
            distance_m,
        });
    }
    Ok(distance_m)
}

fn profile_rails(
    edge_idx: usize,
    side: IncidentEdgeSide,
    profile_kind: NodeInputProfileKind,
    profile: &IncidentMouthProfile,
) -> Vec<NodeInputProfileRail> {
    profile
        .bands
        .iter()
        .enumerate()
        .map(|(band_index, band)| NodeInputProfileRail {
            profile_kind,
            band_index,
            band_kind: band.kind,
            start_world: godot_vec3_to_road(band.start_point_world),
            end_world: godot_vec3_to_road(band.end_point_world),
            height_source: band_height_source(profile_kind, edge_idx, side, band_index),
        })
        .collect()
}

fn boundary_rails(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBoundaryRail> {
    mouth
        .profile
        .boundary_points_world
        .iter()
        .zip(&mouth.endpoint_profile.boundary_points_world)
        .enumerate()
        .map(
            |(boundary_index, (mouth_point, endpoint_point))| NodeInputBoundaryRail {
                boundary_index,
                role: boundary_rail_role(boundary_index, &mouth.profile.bands),
                mouth_world: godot_vec3_to_road(*mouth_point),
                endpoint_world: godot_vec3_to_road(*endpoint_point),
            },
        )
        .collect()
}

fn band_intervals(mouth: &OrderedIncidentPieceMouth) -> Vec<NodeInputBandInterval> {
    mouth
        .profile
        .bands
        .iter()
        .zip(&mouth.endpoint_profile.bands)
        .enumerate()
        .map(
            |(band_index, (mouth_band, endpoint_band))| NodeInputBandInterval {
                band_index,
                band_kind: mouth_band.kind,
                mouth_start_world: godot_vec3_to_road(mouth_band.start_point_world),
                mouth_end_world: godot_vec3_to_road(mouth_band.end_point_world),
                endpoint_start_world: godot_vec3_to_road(endpoint_band.start_point_world),
                endpoint_end_world: godot_vec3_to_road(endpoint_band.end_point_world),
                mouth_height_source: band_height_source(
                    NodeInputProfileKind::Mouth,
                    mouth.edge_idx,
                    mouth.side,
                    band_index,
                ),
                endpoint_height_source: band_height_source(
                    NodeInputProfileKind::Endpoint,
                    mouth.edge_idx,
                    mouth.side,
                    band_index,
                ),
            },
        )
        .collect()
}

fn boundary_heights(
    edge_idx: usize,
    side: IncidentEdgeSide,
    profile_kind: NodeInputProfileKind,
    profile: &IncidentMouthProfile,
) -> Vec<NodeInputBoundaryHeight> {
    profile
        .boundary_points_world
        .iter()
        .enumerate()
        .map(|(boundary_index, point)| NodeInputBoundaryHeight {
            profile_kind,
            boundary_index,
            point_xz: godot_vec3_xz_to_road(*point),
            height_m: f64::from(point.y),
            height_sources: boundary_height_sources(
                profile_kind,
                edge_idx,
                side,
                boundary_index,
                profile.bands.len(),
            ),
        })
        .collect()
}

fn boundary_height_sources(
    profile_kind: NodeInputProfileKind,
    edge_idx: usize,
    side: IncidentEdgeSide,
    boundary_index: usize,
    band_count: usize,
) -> Vec<NodeHeightSource> {
    let mut sources = Vec::with_capacity(2);
    if boundary_index > 0 {
        sources.push(band_height_source(
            profile_kind,
            edge_idx,
            side,
            boundary_index - 1,
        ));
    }
    if boundary_index < band_count {
        sources.push(band_height_source(
            profile_kind,
            edge_idx,
            side,
            boundary_index,
        ));
    }
    sources.sort();
    sources.dedup();
    sources
}

fn band_height_source(
    profile_kind: NodeInputProfileKind,
    edge_idx: usize,
    side: IncidentEdgeSide,
    band_index: usize,
) -> NodeHeightSource {
    match profile_kind {
        NodeInputProfileKind::Mouth => NodeHeightSource::IncidentMouthBand {
            edge_idx,
            side,
            band_index,
        },
        NodeInputProfileKind::Endpoint => NodeHeightSource::EndpointBand {
            edge_idx,
            side,
            band_index,
        },
    }
}

fn boundary_rail_role(
    boundary_index: usize,
    bands: &[IncidentMouthBand],
) -> NodeInputBoundaryRailRole {
    match (
        boundary_index
            .checked_sub(1)
            .and_then(|index| bands.get(index)),
        bands.get(boundary_index),
    ) {
        (None, Some(right_band)) => NodeInputBoundaryRailRole::OuterFootprint {
            adjacent_kind: right_band.kind,
        },
        (Some(left_band), None) => NodeInputBoundaryRailRole::OuterFootprint {
            adjacent_kind: left_band.kind,
        },
        (Some(left_band), Some(right_band)) => NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind: left_band.kind,
            right_kind: right_band.kind,
        },
        (None, None) => unreachable!("validated profile must have at least one band"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot::prelude::{Vector2, Vector3};

    fn test_band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn test_profile(x: f32, direction: Vector2) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -2.0),
            Vector3::new(x, 4.2, 0.0),
            Vector3::new(x, 4.3, 2.0),
            Vector3::new(x, 4.4, 4.0),
        ];
        let bands = vec![
            test_band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            test_band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            test_band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            test_band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: direction,
            boundary_points_world,
            bands,
        }
    }

    fn test_mouth() -> OrderedIncidentPieceMouth {
        OrderedIncidentPieceMouth {
            profile: test_profile(10.0, Vector2::RIGHT),
            endpoint_profile: test_profile(0.0, Vector2::RIGHT),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        }
    }

    #[test]
    fn extracts_profile_rails_intervals_boundary_heights_and_handoff() {
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[test_mouth()],
        )
        .expect("valid solved profiles should produce canonical input");

        assert_eq!(input.node_id, 42);
        assert_eq!(input.piece_kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert_eq!(input.mouths.len(), 1);

        let mouth = &input.mouths[0];
        assert_eq!(mouth.order_index, 0);
        assert_eq!(mouth.edge_idx, 7);
        assert_eq!(mouth.side, IncidentEdgeSide::Start);
        assert_eq!(mouth.mouth_rails.len(), 4);
        assert_eq!(mouth.endpoint_rails.len(), 4);
        assert_eq!(mouth.boundary_rails.len(), 5);
        assert_eq!(mouth.band_intervals.len(), 4);
        assert_eq!(mouth.boundary_heights.len(), 10);
        assert!((mouth.conflict_handoff_distance_m - 10.0).abs() <= f64::EPSILON);
        assert_eq!(
            mouth.boundary_rails[0].role,
            NodeInputBoundaryRailRole::OuterFootprint {
                adjacent_kind: RoadSurfaceBandKind::Sidewalk
            }
        );
        assert_eq!(
            mouth.boundary_rails[2].role,
            NodeInputBoundaryRailRole::InteriorBandBoundary {
                left_kind: RoadSurfaceBandKind::CurbOrShoulder,
                right_kind: RoadSurfaceBandKind::Carriageway,
            }
        );
        assert_eq!(
            mouth.boundary_heights[2].height_sources,
            vec![
                NodeHeightSource::IncidentMouthBand {
                    edge_idx: 7,
                    side: IncidentEdgeSide::Start,
                    band_index: 1,
                },
                NodeHeightSource::IncidentMouthBand {
                    edge_idx: 7,
                    side: IncidentEdgeSide::Start,
                    band_index: 2,
                },
            ]
        );
    }

    #[test]
    fn rejects_mismatched_profile_band_kinds() {
        let mut mouth = test_mouth();
        mouth.endpoint_profile.bands[1].kind = RoadSurfaceBandKind::Median;

        assert_eq!(
            NodeArrangementInput::from_ordered_mouths(
                42,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                &[mouth],
            ),
            Err(NodeInputExtractionError::ProfileBandKindMismatch {
                edge_idx: 7,
                side: IncidentEdgeSide::Start,
                band_index: 1,
                mouth_kind: RoadSurfaceBandKind::CurbOrShoulder,
                endpoint_kind: RoadSurfaceBandKind::Median,
            })
        );
    }

    #[test]
    fn rejects_profile_boundary_count_mismatch() {
        let mut mouth = test_mouth();
        mouth.profile.boundary_points_world.pop();

        assert_eq!(
            NodeArrangementInput::from_ordered_mouths(
                42,
                RoadSurfaceVisualNodePieceKind::JunctionN,
                &[mouth],
            ),
            Err(NodeInputExtractionError::ProfileBoundaryCountMismatch {
                edge_idx: 7,
                side: IncidentEdgeSide::Start,
                profile_kind: NodeInputProfileKind::Mouth,
                expected: 5,
                actual: 4,
            })
        );
    }
}
