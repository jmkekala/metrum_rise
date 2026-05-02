//! Library-backed rail and contour generation for canonical node arrangements.

#![allow(dead_code)]

use super::arrangement::{NodeBandOwner, NodeHeightSource};
use super::backend::{
    RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points, road_points_to_polyline,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRailRole, NodeInputMouth,
    NodeInputProfileKind, NodeInputProfileRail, NodeInputTerminalEndBand,
};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};

const RAIL_CONTOUR_POINT_EQUAL_EPS_M: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourKind {
    FullRoadbed,
    Band { kind: RoadSurfaceBandKind },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeRailConstraintKind {
    FullRoadbedContour,
    BandContour {
        kind: RoadSurfaceBandKind,
    },
    SpanHandoff {
        kind: RoadSurfaceBandKind,
    },
    FootprintSeam {
        adjacent_kind: RoadSurfaceBandKind,
    },
    AsphaltBoundary {
        adjacent_kind: RoadSurfaceBandKind,
    },
    AsphaltCurbContact,
    CurbSidewalkContact,
    BandBoundary {
        left_kind: RoadSurfaceBandKind,
        right_kind: RoadSurfaceBandKind,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct NodeRailContourSet {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) contours: Vec<NodeGeneratedContour>,
    pub(crate) constraints: Vec<NodeRailConstraint>,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeGeneratedContour {
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) backend_polyline: RoadPolyline,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeRailConstraint {
    pub(crate) constraint_index: usize,
    pub(crate) kind: NodeRailConstraintKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) source_boundary_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) opposite_owner: Option<NodeBandOwner>,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeRailGenerationError {
    EmptyInput {
        node_id: u32,
    },
    DegenerateContour {
        kind: NodeGeneratedContourKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        area_m2: f64,
        vertex_count: usize,
    },
    DegenerateConstraint {
        kind: NodeRailConstraintKind,
        mouth_order_index: usize,
        band_index: Option<usize>,
        boundary_index: Option<usize>,
        path_length_m: f64,
        vertex_count: usize,
    },
}

struct MouthOwners {
    band_owners: Vec<NodeBandOwner>,
    terminal_end_band_owners: Vec<NodeBandOwner>,
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_rail_contours_from_input(
        input: &NodeArrangementInput,
    ) -> Result<NodeRailContourSet, NodeRailGenerationError> {
        NodeRailContourSet::from_input(input)
    }
}

impl NodeRailContourSet {
    pub(crate) fn from_input(
        input: &NodeArrangementInput,
    ) -> Result<Self, NodeRailGenerationError> {
        if input.mouths.is_empty() {
            return Err(NodeRailGenerationError::EmptyInput {
                node_id: input.node_id,
            });
        }

        let owners_by_mouth = owners_by_mouth(input);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        for (mouth, mouth_owners) in input.mouths.iter().zip(&owners_by_mouth) {
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;

            for (band_index, interval) in mouth.band_intervals.iter().enumerate() {
                let owner = mouth_owners.band_owners[band_index];
                push_band_contour(mouth, interval, owner, &mut contours, &mut constraints)?;
            }

            for (end_band, owner) in mouth
                .terminal_end_bands
                .iter()
                .zip(&mouth_owners.terminal_end_band_owners)
            {
                push_terminal_end_band_contour(
                    mouth,
                    end_band,
                    *owner,
                    &mut contours,
                    &mut constraints,
                )?;
            }

            for boundary_rail in &mouth.boundary_rails {
                let (owner, opposite_owner) =
                    boundary_owners(boundary_rail.boundary_index, &mouth_owners.band_owners);
                push_boundary_constraint(
                    mouth,
                    boundary_rail.boundary_index,
                    boundary_rail.role,
                    owner,
                    opposite_owner,
                    &mut constraints,
                )?;
            }

            for profile_rail in &mouth.mouth_rails {
                let owner = mouth_owners.band_owners[profile_rail.band_index];
                push_span_handoff_constraint(mouth, profile_rail, owner, &mut constraints)?;
            }
        }

        Ok(Self {
            node_id: input.node_id,
            piece_kind: input.piece_kind,
            contours,
            constraints,
        })
    }
}

fn push_full_roadbed_contour(
    mouth: &NodeInputMouth,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let first = mouth
        .boundary_rails
        .first()
        .expect("validated input has rails");
    let last = mouth
        .boundary_rails
        .last()
        .expect("validated input has rails");
    let points = vec![
        xz(first.mouth_world),
        xz(last.mouth_world),
        xz(last.endpoint_world),
        xz(first.endpoint_world),
    ];
    let contour = cleaned_closed_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        points,
    )?;
    let points_xz = polyline_to_road_points(&contour);
    let height_sources = all_boundary_height_sources(mouth);
    contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        source_mouth_order_index: mouth.order_index,
        source_band_index: None,
        owner: None,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
        height_sources: height_sources.clone(),
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::FullRoadbedContour,
        mouth.order_index,
        None,
        None,
        None,
        None,
        points_xz,
        height_sources,
    )
}

fn push_band_contour(
    mouth: &NodeInputMouth,
    interval: &NodeInputBandInterval,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let kind = NodeGeneratedContourKind::Band {
        kind: interval.band_kind,
    };
    let points = vec![
        xz(interval.mouth_start_world),
        xz(interval.mouth_end_world),
        xz(interval.endpoint_end_world),
        xz(interval.endpoint_start_world),
    ];
    let contour =
        cleaned_closed_contour(kind, mouth.order_index, Some(interval.band_index), points)?;
    let points_xz = polyline_to_road_points(&contour);
    let height_sources = vec![
        interval.mouth_height_source.clone(),
        interval.endpoint_height_source.clone(),
    ];
    contours.push(NodeGeneratedContour {
        kind,
        source_mouth_order_index: mouth.order_index,
        source_band_index: Some(interval.band_index),
        owner: Some(owner),
        points_xz: points_xz.clone(),
        backend_polyline: contour,
        height_sources: height_sources.clone(),
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::BandContour {
            kind: interval.band_kind,
        },
        mouth.order_index,
        Some(interval.band_index),
        None,
        Some(owner),
        None,
        points_xz,
        height_sources,
    )
}

fn push_terminal_end_band_contour(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let points = vec![
        xz(end_band.inner_start_world),
        xz(end_band.inner_end_world),
        xz(end_band.outer_end_world),
        xz(end_band.outer_start_world),
    ];
    let footprint = cleaned_closed_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        points.clone(),
    )?;
    let footprint_points_xz = polyline_to_road_points(&footprint);
    let height_sources = canonical_height_sources(end_band.height_sources.iter().cloned());
    contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        source_mouth_order_index: mouth.order_index,
        source_band_index: None,
        owner: None,
        points_xz: footprint_points_xz.clone(),
        backend_polyline: footprint,
        height_sources: height_sources.clone(),
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::FullRoadbedContour,
        mouth.order_index,
        None,
        None,
        None,
        None,
        footprint_points_xz,
        height_sources.clone(),
    )?;

    let kind = NodeGeneratedContourKind::Band {
        kind: end_band.band_kind,
    };
    let contour = cleaned_closed_contour(
        kind,
        mouth.order_index,
        Some(end_band.source_band_index),
        points,
    )?;
    let points_xz = polyline_to_road_points(&contour);
    contours.push(NodeGeneratedContour {
        kind,
        source_mouth_order_index: mouth.order_index,
        source_band_index: Some(end_band.source_band_index),
        owner: Some(owner),
        points_xz: points_xz.clone(),
        backend_polyline: contour,
        height_sources: height_sources.clone(),
    });
    push_constraint(
        constraints,
        NodeRailConstraintKind::BandContour {
            kind: end_band.band_kind,
        },
        mouth.order_index,
        Some(end_band.source_band_index),
        None,
        Some(owner),
        None,
        points_xz,
        height_sources,
    )
}

fn push_boundary_constraint(
    mouth: &NodeInputMouth,
    boundary_index: usize,
    role: NodeInputBoundaryRailRole,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let rail = &mouth.boundary_rails[boundary_index];
    push_constraint(
        constraints,
        boundary_constraint_kind(role),
        mouth.order_index,
        None,
        Some(boundary_index),
        owner,
        opposite_owner,
        vec![xz(rail.mouth_world), xz(rail.endpoint_world)],
        boundary_height_sources(mouth, boundary_index),
    )
}

fn push_span_handoff_constraint(
    mouth: &NodeInputMouth,
    profile_rail: &NodeInputProfileRail,
    owner: NodeBandOwner,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    push_constraint(
        constraints,
        NodeRailConstraintKind::SpanHandoff {
            kind: profile_rail.band_kind,
        },
        mouth.order_index,
        Some(profile_rail.band_index),
        None,
        Some(owner),
        None,
        vec![xz(profile_rail.start_world), xz(profile_rail.end_world)],
        vec![profile_rail.height_source.clone()],
    )
}

fn push_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    source_boundary_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
    height_sources: Vec<NodeHeightSource>,
) -> Result<(), NodeRailGenerationError> {
    let polyline = cleaned_open_rail(
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        points,
    )?;
    let constraint_index = constraints.len();
    constraints.push(NodeRailConstraint {
        constraint_index,
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index,
        owner,
        opposite_owner,
        points_xz: polyline_to_road_points(&polyline),
        height_sources: canonical_height_sources(height_sources),
    });
    Ok(())
}

fn cleaned_closed_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    points: Vec<RoadVec2>,
) -> Result<RoadPolyline, NodeRailGenerationError> {
    let raw = road_points_to_polyline(points, true);
    let mut contour = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
    if contour.area() < 0.0 {
        contour.invert_direction_mut();
    }
    let area_m2 = contour.area().abs();
    if contour.vertex_count() < 3 || area_m2 <= f64::from(NODE_OVERLAY_MIN_AREA_M2) {
        return Err(NodeRailGenerationError::DegenerateContour {
            kind,
            mouth_order_index,
            band_index,
            area_m2,
            vertex_count: contour.vertex_count(),
        });
    }
    Ok(contour)
}

fn cleaned_open_rail(
    kind: NodeRailConstraintKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    boundary_index: Option<usize>,
    points: Vec<RoadVec2>,
) -> Result<RoadPolyline, NodeRailGenerationError> {
    let raw = road_points_to_polyline(points, false);
    let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
    let path_length_m = rail.path_length();
    if rail.vertex_count() < 2 || path_length_m <= RAIL_CONTOUR_POINT_EQUAL_EPS_M {
        return Err(NodeRailGenerationError::DegenerateConstraint {
            kind,
            mouth_order_index,
            band_index,
            boundary_index,
            path_length_m,
            vertex_count: rail.vertex_count(),
        });
    }
    Ok(rail)
}

fn owners_by_mouth(input: &NodeArrangementInput) -> Vec<MouthOwners> {
    let mut next_owner_index = 0usize;
    input
        .mouths
        .iter()
        .map(|mouth| {
            let band_owners = mouth
                .band_intervals
                .iter()
                .map(|interval| {
                    let owner = NodeBandOwner::new(interval.band_kind, next_owner_index);
                    next_owner_index += 1;
                    owner
                })
                .collect();
            let terminal_end_band_owners = mouth
                .terminal_end_bands
                .iter()
                .map(|end_band| {
                    let owner = NodeBandOwner::new(end_band.band_kind, next_owner_index);
                    next_owner_index += 1;
                    owner
                })
                .collect();
            MouthOwners {
                band_owners,
                terminal_end_band_owners,
            }
        })
        .collect()
}

fn boundary_owners(
    boundary_index: usize,
    band_owners: &[NodeBandOwner],
) -> (Option<NodeBandOwner>, Option<NodeBandOwner>) {
    let left_owner = boundary_index
        .checked_sub(1)
        .and_then(|index| band_owners.get(index))
        .copied();
    let right_owner = band_owners.get(boundary_index).copied();
    match (left_owner, right_owner) {
        (Some(left_owner), Some(right_owner)) => (Some(left_owner), Some(right_owner)),
        (Some(owner), None) | (None, Some(owner)) => (Some(owner), None),
        (None, None) => (None, None),
    }
}

fn boundary_constraint_kind(role: NodeInputBoundaryRailRole) -> NodeRailConstraintKind {
    match role {
        NodeInputBoundaryRailRole::OuterFootprint { adjacent_kind } => {
            NodeRailConstraintKind::FootprintSeam { adjacent_kind }
        }
        NodeInputBoundaryRailRole::InteriorBandBoundary {
            left_kind,
            right_kind,
        } => {
            if is_carriageway(left_kind) && is_curb_or_shoulder(right_kind)
                || is_curb_or_shoulder(left_kind) && is_carriageway(right_kind)
            {
                NodeRailConstraintKind::AsphaltCurbContact
            } else if is_carriageway(left_kind) || is_carriageway(right_kind) {
                let adjacent_kind = if is_carriageway(left_kind) {
                    right_kind
                } else {
                    left_kind
                };
                NodeRailConstraintKind::AsphaltBoundary { adjacent_kind }
            } else if is_curb_or_shoulder(left_kind) && is_sidewalk(right_kind)
                || is_sidewalk(left_kind) && is_curb_or_shoulder(right_kind)
            {
                NodeRailConstraintKind::CurbSidewalkContact
            } else {
                NodeRailConstraintKind::BandBoundary {
                    left_kind,
                    right_kind,
                }
            }
        }
    }
}

fn boundary_height_sources(mouth: &NodeInputMouth, boundary_index: usize) -> Vec<NodeHeightSource> {
    canonical_height_sources(
        mouth
            .boundary_heights
            .iter()
            .filter(|height| {
                height.boundary_index == boundary_index
                    && matches!(
                        height.profile_kind,
                        NodeInputProfileKind::Mouth | NodeInputProfileKind::Endpoint
                    )
            })
            .flat_map(|height| height.height_sources.iter().cloned()),
    )
}

fn all_boundary_height_sources(mouth: &NodeInputMouth) -> Vec<NodeHeightSource> {
    canonical_height_sources(
        mouth
            .boundary_heights
            .iter()
            .flat_map(|height| height.height_sources.iter().cloned()),
    )
}

fn canonical_height_sources(
    sources: impl IntoIterator<Item = NodeHeightSource>,
) -> Vec<NodeHeightSource> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

fn xz(point: RoadVec3) -> RoadVec2 {
    RoadVec2::new(point.x, point.z)
}

fn is_carriageway(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Carriageway
}

fn is_curb_or_shoulder(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::CurbOrShoulder
}

fn is_sidewalk(kind: RoadSurfaceBandKind) -> bool {
    kind == RoadSurfaceBandKind::Sidewalk
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -2.0),
            Vector3::new(x, 4.2, 0.0),
            Vector3::new(x, 4.3, 2.0),
            Vector3::new(x, 4.4, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0),
            endpoint_profile: profile(endpoint_x),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input")
    }

    #[test]
    fn generates_backend_contours_and_constraints_from_solved_mouth_input() {
        let contours =
            NodeRailContourSet::from_input(&input_with_endpoint_x(0.0)).expect("valid contours");

        assert_eq!(contours.node_id, 42);
        assert_eq!(
            contours.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(contours.contours.len(), 5);
        assert_eq!(contours.constraints.len(), 14);
        assert_eq!(
            contours.contours[0].kind,
            NodeGeneratedContourKind::FullRoadbed
        );
        assert_eq!(contours.contours[0].points_xz.len(), 4);
        assert!(contours.contours.iter().any(|contour| contour.kind
            == NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway
            }));
        assert!(
            contours
                .constraints
                .iter()
                .any(|constraint| constraint.kind == NodeRailConstraintKind::AsphaltCurbContact)
        );
        assert!(
            contours
                .constraints
                .iter()
                .any(|constraint| constraint.kind == NodeRailConstraintKind::CurbSidewalkContact)
        );
        assert_eq!(
            contours.constraints[0].kind,
            NodeRailConstraintKind::FullRoadbedContour
        );
        assert_eq!(contours.constraints[0].constraint_index, 0);
        assert!(!contours.constraints[0].height_sources.is_empty());
    }

    #[test]
    fn rejects_degenerate_backend_contours() {
        let error = NodeRailContourSet::from_input(&input_with_endpoint_x(10.0))
            .expect_err("zero-depth mouth should collapse its contours");

        assert!(matches!(
            error,
            NodeRailGenerationError::DegenerateContour {
                kind: NodeGeneratedContourKind::FullRoadbed,
                mouth_order_index: 0,
                band_index: None,
                ..
            }
        ));
    }
}
