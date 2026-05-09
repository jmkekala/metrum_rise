//! Library-backed rail and contour generation for canonical node arrangements.

use super::arrangement::NodeBandOwner;
use super::backend::{
    ROAD_OVERLAY_COORDINATE_SCALE, RoadPolyline, RoadVec2, RoadVec3, polyline_to_road_points,
    road_points_to_polyline,
};
use super::input::{
    NodeArrangementInput, NodeInputBandInterval, NodeInputBoundaryRailRole, NodeInputMouth,
    NodeInputProfileRail, NodeInputTerminalEndBand, NodeInputTerminalEndBandBoundaryMode,
};
use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeOverlayContour, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use cavalier_contours::polyline::{PlineCreation, PlineSource, PlineSourceMut};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::{BTreeMap, BTreeSet};

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
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) backend_polyline: RoadPolyline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourClaimPriority {
    JoinOrCap,
    MouthBand,
    Footprint,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum GeneratedSameBandBoundaryRole {
    LowerSide,
    RaisedSide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedContourEdgeKey {
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedContourDirectedEdge {
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandContactConstraint {
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedSameBandContactConstraintKey {
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedMaterialPointContactAuthority {
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
}

type NodeRailPointKey = (i64, i64);

impl GeneratedContourEdgeKey {
    fn new(a: NodeRailPointKey, b: NodeRailPointKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

impl GeneratedSameBandContactConstraint {
    fn key(self) -> GeneratedSameBandContactConstraintKey {
        let edge = GeneratedContourEdgeKey::new(self.start, self.end);
        GeneratedSameBandContactConstraintKey {
            kind: self.kind,
            owner: self.owner,
            opposite_owner: self.opposite_owner,
            start: edge.start,
            end: edge.end,
        }
    }
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

            push_terminal_end_band_contours(
                input.piece_kind,
                mouth,
                &mouth.terminal_end_bands,
                &mouth_owners.terminal_end_band_owners,
                &mut contours,
                &mut constraints,
            )?;
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
        append_generated_role_material_contact_constraints(&contours, &mut constraints);
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_generated_material_point_contact_constraints(&contours, &mut constraints);
        append_generated_same_band_contact_constraints(&contours, &mut constraints);

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
    contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        source_mouth_order_index: mouth.order_index,
        source_band_index: None,
        owner: None,
        claim_priority: NodeGeneratedContourClaimPriority::Footprint,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
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
    contours.push(NodeGeneratedContour {
        kind,
        source_mouth_order_index: mouth.order_index,
        source_band_index: Some(interval.band_index),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
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
    )
}

fn push_terminal_end_band_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    end_bands: &[NodeInputTerminalEndBand],
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        for (end_band, owner) in end_bands.iter().zip(owners) {
            push_single_terminal_end_band_contour(mouth, end_band, *owner, contours, constraints)?;
        }
        return Ok(());
    }

    let mut groups = BTreeMap::<TerminalEndBandGroupKey, TerminalEndBandGroup>::new();
    let mut owner_by_kind_and_source = BTreeMap::new();
    for (end_band, owner) in end_bands.iter().zip(owners) {
        owner_by_kind_and_source.insert((end_band.band_kind, end_band.source_band_index), *owner);
        groups
            .entry(TerminalEndBandGroupKey {
                kind: end_band.band_kind,
                source_band_index: end_band.source_band_index,
                owner: *owner,
                contributes_footprint: terminal_end_band_contributes_footprint(end_band),
            })
            .or_insert_with(|| TerminalEndBandGroup {
                contour_world: Vec::new(),
                end_bands: Vec::new(),
            })
            .push(end_band);
    }

    for (key, group) in groups {
        push_terminal_end_band_group_contours(
            mouth,
            key,
            &group.contour_world,
            contours,
            constraints,
        )?;
        for end_band in group.end_bands {
            push_terminal_end_band_boundary_constraints(
                mouth,
                end_band,
                key.owner,
                &owner_by_kind_and_source,
                constraints,
            )?;
        }
    }

    Ok(())
}

fn push_single_terminal_end_band_contour(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let points = end_band
        .contour_world
        .iter()
        .copied()
        .map(xz)
        .collect::<Vec<_>>();
    if terminal_end_band_contributes_footprint(end_band) {
        let footprint = cleaned_closed_contour(
            NodeGeneratedContourKind::FullRoadbed,
            mouth.order_index,
            None,
            points.clone(),
        )?;
        let footprint_points_xz = polyline_to_road_points(&footprint);
        contours.push(NodeGeneratedContour {
            kind: NodeGeneratedContourKind::FullRoadbed,
            source_mouth_order_index: mouth.order_index,
            source_band_index: None,
            owner: None,
            claim_priority: NodeGeneratedContourClaimPriority::Footprint,
            points_xz: footprint_points_xz.clone(),
            backend_polyline: footprint,
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
        )?;
    }

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
        claim_priority: NodeGeneratedContourClaimPriority::JoinOrCap,
        points_xz: points_xz.clone(),
        backend_polyline: contour,
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
    )?;

    push_terminal_end_band_boundary_constraints(
        mouth,
        end_band,
        owner,
        &BTreeMap::new(),
        constraints,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct TerminalEndBandGroupKey {
    kind: RoadSurfaceBandKind,
    source_band_index: usize,
    owner: NodeBandOwner,
    contributes_footprint: bool,
}

struct TerminalEndBandGroup<'a> {
    contour_world: Vec<NodeOverlayContour>,
    end_bands: Vec<&'a NodeInputTerminalEndBand>,
}

impl<'a> TerminalEndBandGroup<'a> {
    fn push(&mut self, end_band: &'a NodeInputTerminalEndBand) {
        self.contour_world.push(
            end_band
                .contour_world
                .iter()
                .map(|point| [point.x, point.z])
                .collect(),
        );
        self.end_bands.push(end_band);
    }
}

fn push_terminal_end_band_group_contours(
    mouth: &NodeInputMouth,
    key: TerminalEndBandGroupKey,
    contour_world: &[NodeOverlayContour],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(mut shapes) = RoadSurfaceSystem::overlay_union_contours(contour_world) else {
        return Ok(());
    };
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);

    for shape in shapes {
        for contour in shape {
            let points = contour
                .into_iter()
                .map(|point| RoadVec2::new(point[0], point[1]))
                .collect::<Vec<_>>();
            if key.contributes_footprint {
                let footprint = cleaned_closed_contour(
                    NodeGeneratedContourKind::FullRoadbed,
                    mouth.order_index,
                    None,
                    points.clone(),
                )?;
                let footprint_points_xz = polyline_to_road_points(&footprint);
                contours.push(NodeGeneratedContour {
                    kind: NodeGeneratedContourKind::FullRoadbed,
                    source_mouth_order_index: mouth.order_index,
                    source_band_index: None,
                    owner: None,
                    claim_priority: NodeGeneratedContourClaimPriority::Footprint,
                    points_xz: footprint_points_xz.clone(),
                    backend_polyline: footprint,
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
                )?;
            }

            let kind = NodeGeneratedContourKind::Band { kind: key.kind };
            let band_contour = cleaned_closed_contour(
                kind,
                mouth.order_index,
                Some(key.source_band_index),
                points,
            )?;
            let points_xz = polyline_to_road_points(&band_contour);
            contours.push(NodeGeneratedContour {
                kind,
                source_mouth_order_index: mouth.order_index,
                source_band_index: Some(key.source_band_index),
                owner: Some(key.owner),
                claim_priority: NodeGeneratedContourClaimPriority::JoinOrCap,
                points_xz: points_xz.clone(),
                backend_polyline: band_contour,
            });
            push_constraint(
                constraints,
                NodeRailConstraintKind::BandContour { kind: key.kind },
                mouth.order_index,
                Some(key.source_band_index),
                None,
                Some(key.owner),
                None,
                points_xz,
            )?;
        }
    }

    Ok(())
}

fn push_terminal_end_band_boundary_constraints(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let inner_path = terminal_end_band_inner_contour_path(end_band);
    let outer_path = terminal_end_band_outer_contour_path(end_band);
    let outer_cap_path = terminal_end_band_outer_cap_contour_path(end_band);
    let opposite_owner =
        terminal_end_band_material_opposite_owner(end_band, owner_by_kind_and_source);
    match end_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
                && let Some(points) = inner_path
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::AsphaltCurbContact,
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    None,
                    points,
                )?;
            }
            if terminal_end_band_has_material_boundary(end_band)
                && let Some(points) = outer_path.clone()
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::CurbSidewalkContact,
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if terminal_end_band_has_material_boundary(end_band) {
                for (start, end) in terminal_curb_sidewalk_side_edges(end_band) {
                    push_terminal_end_band_constraint(
                        constraints,
                        NodeRailConstraintKind::CurbSidewalkContact,
                        mouth.order_index,
                        end_band.source_band_index,
                        owner,
                        opposite_owner,
                        xz(start),
                        xz(end),
                    )?;
                }
            }
            if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap {
                push_terminal_end_band_cap_role_constraints(
                    mouth,
                    end_band,
                    owner,
                    opposite_owner,
                    constraints,
                )?;
            }
            Ok(())
        }
        RoadSurfaceBandKind::Sidewalk => {
            if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
                && let Some(points) = inner_path
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::CurbSidewalkContact,
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    opposite_owner,
                    points,
                )?;
            }
            if terminal_end_band_has_material_boundary(end_band)
                && let Some(points) = outer_path
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    None,
                    points,
                )?;
            }
            if end_band.boundary_mode == NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
                && let Some(points) = outer_cap_path
            {
                push_terminal_end_band_path_constraint(
                    constraints,
                    NodeRailConstraintKind::FootprintSeam {
                        adjacent_kind: RoadSurfaceBandKind::Sidewalk,
                    },
                    mouth.order_index,
                    end_band.source_band_index,
                    owner,
                    None,
                    points,
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn terminal_end_band_contributes_footprint(end_band: &NodeInputTerminalEndBand) -> bool {
    end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
}

fn terminal_end_band_material_opposite_owner(
    end_band: &NodeInputTerminalEndBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand {
        return None;
    }
    match end_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            end_band
                .source_band_index
                .checked_add(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::Sidewalk, source_band_index))
                        .copied()
                })
        }
        RoadSurfaceBandKind::Sidewalk => {
            end_band
                .source_band_index
                .checked_sub(1)
                .and_then(|source_band_index| {
                    owner_by_kind_and_source
                        .get(&(RoadSurfaceBandKind::CurbOrShoulder, source_band_index))
                        .copied()
                })
        }
        _ => None,
    }
}

fn terminal_end_band_has_material_boundary(end_band: &NodeInputTerminalEndBand) -> bool {
    matches!(
        end_band.boundary_mode,
        NodeInputTerminalEndBandBoundaryMode::MaterialBand
            | NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand
            | NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
    )
}

fn push_terminal_end_band_cap_role_constraints(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    for (start, end) in terminal_curb_sidewalk_side_edges(end_band) {
        push_terminal_end_band_constraint(
            constraints,
            NodeRailConstraintKind::CurbSidewalkContact,
            mouth.order_index,
            end_band.source_band_index,
            owner,
            opposite_owner,
            xz(start),
            xz(end),
        )?;
    }

    Ok(())
}

fn terminal_curb_sidewalk_side_edges(
    end_band: &NodeInputTerminalEndBand,
) -> Vec<(RoadVec3, RoadVec3)> {
    [
        (end_band.inner_start_world, end_band.outer_start_world),
        (end_band.inner_end_world, end_band.outer_end_world),
    ]
    .into_iter()
    .filter(|(start, end)| road_point_key(xz(*start)) != road_point_key(xz(*end)))
    .collect()
}

fn terminal_end_band_inner_contour_path(
    end_band: &NodeInputTerminalEndBand,
) -> Option<Vec<RoadVec2>> {
    if end_band.contour_world.len() < 3 {
        return None;
    }
    let points = if end_band.boundary_mode
        == NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand
        && end_band.contour_world.len() > 4
        && end_band.contour_world.len() % 2 == 0
    {
        end_band
            .contour_world
            .iter()
            .copied()
            .take(end_band.contour_world.len() / 2)
            .map(xz)
            .collect::<Vec<_>>()
    } else {
        vec![xz(end_band.contour_world[0]), xz(end_band.contour_world[1])]
    };
    clean_terminal_constraint_path(points)
}

fn terminal_end_band_outer_contour_path(
    end_band: &NodeInputTerminalEndBand,
) -> Option<Vec<RoadVec2>> {
    if end_band.contour_world.len() < 3 {
        return None;
    }
    let points = if end_band.boundary_mode
        == NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand
        && end_band.contour_world.len() > 4
        && end_band.contour_world.len() % 2 == 0
    {
        end_band
            .contour_world
            .iter()
            .copied()
            .skip(end_band.contour_world.len() / 2)
            .rev()
            .map(xz)
            .collect::<Vec<_>>()
    } else {
        end_band
            .contour_world
            .iter()
            .copied()
            .skip(2)
            .rev()
            .map(xz)
            .collect::<Vec<_>>()
    };
    clean_terminal_constraint_path(points)
}

fn terminal_end_band_outer_cap_contour_path(
    end_band: &NodeInputTerminalEndBand,
) -> Option<Vec<RoadVec2>> {
    if end_band.contour_world.len() < 3 {
        return None;
    }
    let mut points = end_band
        .contour_world
        .iter()
        .copied()
        .skip(1)
        .map(xz)
        .collect::<Vec<_>>();
    points.push(xz(end_band.contour_world[0]));
    clean_terminal_constraint_path(points)
}

fn push_terminal_end_band_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    start: RoadVec2,
    end: RoadVec2,
) -> Result<(), NodeRailGenerationError> {
    if road_point_key(start) == road_point_key(end) {
        return Ok(());
    }
    push_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        opposite_owner,
        vec![start, end],
    )
}

fn push_terminal_end_band_path_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    source_mouth_order_index: usize,
    source_band_index: usize,
    owner: NodeBandOwner,
    opposite_owner: Option<NodeBandOwner>,
    points: Vec<RoadVec2>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = clean_terminal_constraint_path(points) else {
        return Ok(());
    };
    push_constraint(
        constraints,
        kind,
        source_mouth_order_index,
        Some(source_band_index),
        None,
        Some(owner),
        opposite_owner,
        points,
    )
}

fn clean_terminal_constraint_path(points: Vec<RoadVec2>) -> Option<Vec<RoadVec2>> {
    let mut cleaned = Vec::with_capacity(points.len());
    for point in points {
        if cleaned
            .last()
            .is_none_or(|last| road_point_key(*last) != road_point_key(point))
        {
            cleaned.push(point);
        }
    }
    if cleaned
        .windows(2)
        .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        Some(cleaned)
    } else {
        None
    }
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
    });
    Ok(())
}

fn insert_generated_material_point_constraint(
    constraints: &mut Vec<NodeRailConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    point: NodeRailPointKey,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
) {
    let edge = [road_point_from_key(point), road_point_from_key(point)];
    if constraints.iter().any(|constraint| {
        constraint.kind == kind
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                owner,
                opposite_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == point
            && road_point_key(constraint.points_xz[1]) == point
    }) {
        return;
    }
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind,
        source_mouth_order_index,
        source_band_index,
        source_boundary_index: None,
        owner: Some(owner),
        opposite_owner: Some(opposite_owner),
        points_xz: edge.to_vec(),
    });
}

fn append_generated_role_material_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let role_constraints = constraints.to_vec();
    let mut contacts = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint)
        .collect::<BTreeSet<_>>();
    append_generated_role_pair_material_contacts(&role_constraints, &mut contacts);

    for role in role_constraints
        .iter()
        .filter(|constraint| generated_material_role_constraint_kind(constraint.kind).is_some())
    {
        let Some(source_owner) = role.owner else {
            continue;
        };
        if !is_curb_or_shoulder(source_owner.kind()) {
            continue;
        }
        if role.opposite_owner.is_some() {
            continue;
        }
        let Some(contact_kind) = generated_material_role_constraint_kind(role.kind) else {
            continue;
        };
        for target in contours {
            let Some(target_owner) = target.owner else {
                continue;
            };
            if target_owner == source_owner
                || generated_contact_constraint_kind(source_owner.kind(), target_owner.kind())
                    != Some(contact_kind)
            {
                continue;
            }
            for contact in generated_role_material_contacts_for_contour(
                role,
                contact_kind,
                source_owner,
                target_owner,
                target,
            ) {
                contacts.insert(contact);
            }
        }
    }

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contacts {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: contact.kind,
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![
                road_point_from_key(contact.start),
                road_point_from_key(contact.end),
            ],
        });
    }
}

fn append_generated_role_pair_material_contacts(
    constraints: &[NodeRailConstraint],
    contacts: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let roles = constraints
        .iter()
        .filter_map(generated_one_sided_material_role_constraint)
        .collect::<Vec<_>>();
    for left_index in 0..roles.len() {
        let left = roles[left_index];
        for right in roles.iter().copied().skip(left_index + 1) {
            if left.kind != right.kind || left.owner == right.owner {
                continue;
            }
            if generated_contact_constraint_kind(left.owner.kind(), right.owner.kind())
                != Some(left.kind)
            {
                continue;
            }
            let source = if left.constraint.constraint_index <= right.constraint.constraint_index {
                left.constraint
            } else {
                right.constraint
            };
            for left_edge in generated_constraint_directed_edges(left.constraint) {
                for right_edge in generated_constraint_directed_edges(right.constraint) {
                    if let Some(edge) = generated_segment_overlap_edge(
                        left_edge.start,
                        left_edge.end,
                        right_edge.start,
                        right_edge.end,
                    ) {
                        contacts.insert(generated_material_contact_constraint(
                            left.kind,
                            left.owner,
                            right.owner,
                            edge.start,
                            edge.end,
                            source,
                        ));
                        continue;
                    }
                    for point in generated_segment_touch_points(
                        left_edge.start,
                        left_edge.end,
                        right_edge.start,
                        right_edge.end,
                    ) {
                        contacts.insert(generated_material_contact_constraint(
                            left.kind,
                            left.owner,
                            right.owner,
                            point,
                            point,
                            source,
                        ));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct GeneratedOneSidedMaterialRoleConstraint<'a> {
    constraint: &'a NodeRailConstraint,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
}

fn generated_one_sided_material_role_constraint(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedOneSidedMaterialRoleConstraint<'_>> {
    if constraint.opposite_owner.is_some() {
        return None;
    }
    Some(GeneratedOneSidedMaterialRoleConstraint {
        constraint,
        kind: generated_material_role_constraint_kind(constraint.kind)?,
        owner: constraint.owner?,
    })
}

fn generated_role_material_contacts_for_contour(
    role: &NodeRailConstraint,
    kind: NodeRailConstraintKind,
    source_owner: NodeBandOwner,
    target_owner: NodeBandOwner,
    target: &NodeGeneratedContour,
) -> Vec<GeneratedSameBandContactConstraint> {
    let mut contacts = BTreeSet::new();
    for role_edge in generated_constraint_directed_edges(role) {
        if is_curb_or_shoulder(source_owner.kind()) {
            for edge in generated_role_edge_segments_inside_contour(role_edge, target) {
                contacts.insert(generated_material_contact_constraint(
                    kind,
                    source_owner,
                    target_owner,
                    edge.start,
                    edge.end,
                    role,
                ));
            }
        }
        for target_edge in generated_contour_directed_edges(target) {
            if let Some(edge) = generated_segment_overlap_edge(
                role_edge.start,
                role_edge.end,
                target_edge.start,
                target_edge.end,
            ) {
                contacts.insert(generated_material_contact_constraint(
                    kind,
                    source_owner,
                    target_owner,
                    edge.start,
                    edge.end,
                    role,
                ));
                continue;
            }
            for point in generated_segment_touch_points(
                role_edge.start,
                role_edge.end,
                target_edge.start,
                target_edge.end,
            ) {
                contacts.insert(generated_material_contact_constraint(
                    kind,
                    source_owner,
                    target_owner,
                    point,
                    point,
                    role,
                ));
            }
        }
    }
    contacts.into_iter().collect()
}

fn generated_role_edge_segments_inside_contour(
    role_edge: GeneratedContourDirectedEdge,
    target: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![role_edge.start, role_edge.end];
    for target_edge in generated_contour_directed_edges(target) {
        if let Some(point) = quantized_proper_segment_intersection(
            role_edge.start,
            role_edge.end,
            target_edge.start,
            target_edge.end,
        ) {
            keys.push(point);
        }
        for point in [target_edge.start, target_edge.end] {
            if generated_point_key_lies_on_segment(point, role_edge.start, role_edge.end) {
                keys.push(point);
            }
        }
        for point in [role_edge.start, role_edge.end] {
            if generated_point_key_lies_on_segment(point, target_edge.start, target_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| {
        generated_segment_parameter_key(role_edge.start, role_edge.end, *point)
    });
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_generated_contour(point_x2, point_z2, target) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

fn generated_material_contact_constraint(
    kind: NodeRailConstraintKind,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    source: &NodeRailConstraint,
) -> GeneratedSameBandContactConstraint {
    let (owner, opposite_owner) = if left_owner <= right_owner {
        (left_owner, right_owner)
    } else {
        (right_owner, left_owner)
    };
    GeneratedSameBandContactConstraint {
        kind,
        owner,
        opposite_owner,
        start,
        end,
        source_mouth_order_index: source.source_mouth_order_index,
        source_band_index: source.source_band_index,
    }
}

fn generated_material_role_constraint_kind(
    kind: NodeRailConstraintKind,
) -> Option<NodeRailConstraintKind> {
    match kind {
        NodeRailConstraintKind::AsphaltCurbContact
        | NodeRailConstraintKind::CurbSidewalkContact => Some(kind),
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::FootprintSeam { .. }
        | NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::BandBoundary { .. } => None,
    }
}

fn append_generated_material_point_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_points = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(left_kind) = generated_contour_band_kind(left) else {
                continue;
            };
            let Some(right_kind) = generated_contour_band_kind(right) else {
                continue;
            };
            if left_kind == right_kind {
                continue;
            }
            let Some(contact_kind) = generated_contact_constraint_kind(left_kind, right_kind)
            else {
                continue;
            };
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let mut points = shared_generated_contour_points(left, right);
            points.extend(generated_contact_points_from_contour_intersections(
                left, right,
            ));
            points.extend(generated_material_authority_points_on_counterpart_contour(
                contact_kind,
                left,
                right,
                left_owner,
                right_owner,
                constraints,
            ));
            points.extend(generated_contour_keys(left).into_iter().filter(|point| {
                generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    *point,
                    constraints,
                )
                .is_some_and(|authority| {
                    authority.owner == Some(right_owner)
                        || authority.opposite_owner == Some(right_owner)
                        || owners_match_unordered(
                            authority.owner,
                            authority.opposite_owner,
                            left_owner,
                            right_owner,
                        )
                })
            }));
            points.extend(generated_contour_keys(right).into_iter().filter(|point| {
                generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    *point,
                    constraints,
                )
                .is_some_and(|authority| {
                    authority.owner == Some(left_owner)
                        || authority.opposite_owner == Some(left_owner)
                        || owners_match_unordered(
                            authority.owner,
                            authority.opposite_owner,
                            left_owner,
                            right_owner,
                        )
                })
            }));
            points.sort_unstable();
            points.dedup();
            for point in points {
                let Some(authority) = generated_material_point_contact_authority(
                    contact_kind,
                    left_owner,
                    right_owner,
                    point,
                    constraints,
                ) else {
                    continue;
                };
                let (owner, opposite_owner) = if left_owner <= right_owner {
                    (left_owner, right_owner)
                } else {
                    (right_owner, left_owner)
                };
                contact_points.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner,
                    opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: authority.source_mouth_order_index,
                    source_band_index: authority.source_band_index,
                });
            }
        }
    }

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contact_points {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        insert_generated_material_point_constraint(
            constraints,
            contact.kind,
            contact.owner,
            contact.opposite_owner,
            contact.start,
            contact.source_mouth_order_index,
            contact.source_band_index,
        );
    }
}

fn generated_material_authority_points_on_counterpart_contour(
    kind: NodeRailConstraintKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    constraints: &[NodeRailConstraint],
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint in constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
    {
        let left_has_authority =
            constraint.owner == Some(left_owner) || constraint.opposite_owner == Some(left_owner);
        let right_has_authority =
            constraint.owner == Some(right_owner) || constraint.opposite_owner == Some(right_owner);
        if !left_has_authority && !right_has_authority {
            continue;
        }
        for point in constraint.points_xz.iter().copied().map(road_point_key) {
            if left_has_authority && generated_contour_contains_key(right, point) {
                points.push(point);
            }
            if right_has_authority && generated_contour_contains_key(left, point) {
                points.push(point);
            }
        }
        if left_has_authority {
            points.extend(generated_constraint_contour_contact_points(
                constraint, right,
            ));
        }
        if right_has_authority {
            points.extend(generated_constraint_contour_contact_points(
                constraint, left,
            ));
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_constraint_contour_contact_points(
    constraint: &NodeRailConstraint,
    contour: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for constraint_edge in generated_constraint_directed_edges(constraint) {
        for contour_edge in generated_contour_directed_edges(contour) {
            if let Some(point) = quantized_proper_segment_intersection(
                constraint_edge.start,
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.start,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.start);
            }
            if generated_point_key_lies_on_segment(
                constraint_edge.end,
                contour_edge.start,
                contour_edge.end,
            ) {
                points.push(constraint_edge.end);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.start,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.start);
            }
            if generated_point_key_lies_on_segment(
                contour_edge.end,
                constraint_edge.start,
                constraint_edge.end,
            ) {
                points.push(contour_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_material_point_contact_authority(
    kind: NodeRailConstraintKind,
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    point: NodeRailPointKey,
    constraints: &[NodeRailConstraint],
) -> Option<GeneratedMaterialPointContactAuthority> {
    constraints
        .iter()
        .filter(|constraint| constraint.kind == kind)
        .filter(|constraint| generated_constraint_touches_key(constraint, point))
        .filter(|constraint| {
            owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                left_owner,
                right_owner,
            ) || constraint.owner == Some(left_owner)
                || constraint.owner == Some(right_owner)
                || constraint.opposite_owner == Some(left_owner)
                || constraint.opposite_owner == Some(right_owner)
        })
        .filter(|constraint| constraint.owner.is_some() || constraint.opposite_owner.is_some())
        .min_by_key(|constraint| {
            (
                !owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    left_owner,
                    right_owner,
                ),
                constraint.constraint_index,
            )
        })
        .map(|constraint| GeneratedMaterialPointContactAuthority {
            source_mouth_order_index: constraint.source_mouth_order_index,
            source_band_index: constraint.source_band_index,
            owner: constraint.owner,
            opposite_owner: constraint.opposite_owner,
        })
}

fn generated_contour_contains_key(contour: &NodeGeneratedContour, point: NodeRailPointKey) -> bool {
    doubled_point_inside_or_on_generated_contour(
        i128::from(point.0) * 2,
        i128::from(point.1) * 2,
        contour,
    )
}

fn append_generated_same_band_contact_constraints(
    contours: &[NodeGeneratedContour],
    constraints: &mut Vec<NodeRailConstraint>,
) {
    let mut contact_edges = BTreeSet::<GeneratedSameBandContactConstraint>::new();
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            let Some(kind) = generated_contour_band_kind(left) else {
                continue;
            };
            let Some(right_kind) = generated_contour_band_kind(right) else {
                continue;
            };
            let Some(contact_kind) = generated_contact_constraint_kind(kind, right_kind) else {
                continue;
            };
            let Some(left_owner) = left.owner else {
                continue;
            };
            let Some(right_owner) = right.owner else {
                continue;
            };
            if left_owner == right_owner {
                continue;
            }
            let (owner, opposite_owner, source_contour) = if left_owner <= right_owner {
                (left_owner, right_owner, left)
            } else {
                (right_owner, left_owner, right)
            };
            let shared_edges = shared_generated_contour_edges(left, right);
            let shared_edge_points = shared_edges
                .iter()
                .flat_map(|edge| [edge.start, edge.end])
                .collect::<BTreeSet<_>>();
            for edge in shared_edges {
                if generated_contact_edge_has_explicit_roles(
                    left,
                    right,
                    constraints,
                    edge,
                    contact_kind,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        owner,
                        opposite_owner,
                        edge,
                        source_contour,
                    );
                }
            }
            for edge in generated_contact_edges_inside_contour(left, right) {
                if generated_contact_edge_has_explicit_roles(
                    left,
                    right,
                    constraints,
                    edge,
                    contact_kind,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        owner,
                        opposite_owner,
                        edge,
                        left,
                    );
                }
            }
            for edge in generated_contact_edges_inside_contour(right, left) {
                if generated_contact_edge_has_explicit_roles(
                    left,
                    right,
                    constraints,
                    edge,
                    contact_kind,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        owner,
                        opposite_owner,
                        edge,
                        right,
                    );
                }
            }
            for edge in generated_contact_edges_from_overlay_intersection(left, right) {
                if generated_contact_edge_has_explicit_roles(
                    left,
                    right,
                    constraints,
                    edge,
                    contact_kind,
                ) {
                    insert_generated_contact_constraint(
                        &mut contact_edges,
                        contact_kind,
                        owner,
                        opposite_owner,
                        edge,
                        source_contour,
                    );
                }
            }
            for point in shared_generated_contour_points(left, right) {
                if shared_edge_points.contains(&point) {
                    continue;
                }
                if !generated_contact_point_has_explicit_roles(
                    kind,
                    right_kind,
                    left,
                    right,
                    constraints,
                    point,
                    contact_kind,
                ) {
                    continue;
                }
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner,
                    opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source_contour.source_mouth_order_index,
                    source_band_index: source_contour.source_band_index,
                });
            }
            for point in generated_contact_points_from_contour_intersections(left, right) {
                if shared_edge_points.contains(&point) {
                    continue;
                }
                if !generated_contact_point_has_explicit_roles(
                    kind,
                    right_kind,
                    left,
                    right,
                    constraints,
                    point,
                    contact_kind,
                ) {
                    continue;
                }
                contact_edges.insert(GeneratedSameBandContactConstraint {
                    kind: contact_kind,
                    owner,
                    opposite_owner,
                    start: point,
                    end: point,
                    source_mouth_order_index: source_contour.source_mouth_order_index,
                    source_band_index: source_contour.source_band_index,
                });
            }
        }
    }

    let mut existing = constraints
        .iter()
        .filter_map(generated_same_band_contact_constraint_key)
        .collect::<BTreeSet<_>>();
    for contact in contact_edges {
        let key = contact.key();
        if !existing.insert(key) {
            continue;
        }
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: contact.kind,
            source_mouth_order_index: contact.source_mouth_order_index,
            source_band_index: contact.source_band_index,
            source_boundary_index: None,
            owner: Some(contact.owner),
            opposite_owner: Some(contact.opposite_owner),
            points_xz: vec![
                road_point_from_key(contact.start),
                road_point_from_key(contact.end),
            ],
        });
    }
}

fn insert_generated_contact_constraint(
    contact_edges: &mut BTreeSet<GeneratedSameBandContactConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    edge: GeneratedContourEdgeKey,
    source_contour: &NodeGeneratedContour,
) {
    for (start, end) in [
        (edge.start, edge.end),
        (edge.start, edge.start),
        (edge.end, edge.end),
    ] {
        contact_edges.insert(GeneratedSameBandContactConstraint {
            kind,
            owner,
            opposite_owner,
            start,
            end,
            source_mouth_order_index: source_contour.source_mouth_order_index,
            source_band_index: source_contour.source_band_index,
        });
    }
}

fn generated_contact_edge_has_explicit_roles(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
    contact_kind: NodeRailConstraintKind,
) -> bool {
    match contact_kind {
        NodeRailConstraintKind::AsphaltCurbContact => {
            generated_curb_contact_role_on_edge(left, right, constraints, edge)
                == Some(GeneratedSameBandBoundaryRole::LowerSide)
        }
        NodeRailConstraintKind::CurbSidewalkContact => {
            let Some(curb_role) =
                generated_curb_contact_role_on_edge(left, right, constraints, edge)
            else {
                return false;
            };
            let Some(sidewalk_role) =
                generated_sidewalk_contact_role_on_edge(left, right, constraints, edge)
            else {
                return false;
            };
            curb_role == GeneratedSameBandBoundaryRole::RaisedSide
                && sidewalk_role == GeneratedSameBandBoundaryRole::LowerSide
        }
        _ => true,
    }
}

fn generated_curb_contact_role_on_edge(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    [left, right]
        .into_iter()
        .find(|contour| generated_contour_band_kind(contour).is_some_and(is_curb_or_shoulder))
        .and_then(|contour| generated_contact_role_on_edge(contour, constraints, edge))
}

fn generated_sidewalk_contact_role_on_edge(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    [left, right]
        .into_iter()
        .find(|contour| generated_contour_band_kind(contour).is_some_and(is_sidewalk))
        .and_then(|contour| generated_contact_role_on_edge(contour, constraints, edge))
}

fn generated_contact_role_on_edge(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    let mut roles = Vec::new();
    collect_generated_same_band_role_on_segment(
        contour,
        constraints,
        edge.start,
        edge.end,
        &mut roles,
    );
    roles.sort_unstable();
    roles.dedup();
    if roles.len() == 1 {
        return roles.first().copied();
    }

    let start_role =
        generated_same_band_boundary_role_at_contour_vertex(contour, constraints, edge.start);
    let end_role =
        generated_same_band_boundary_role_at_contour_vertex(contour, constraints, edge.end);
    match (start_role, end_role) {
        (Some(start_role), Some(end_role)) if start_role == end_role => Some(start_role),
        _ => None,
    }
}

fn generated_contact_edges_inside_contour(
    edge_contour: &NodeGeneratedContour,
    containing_contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    generated_contour_edges(edge_contour)
        .into_iter()
        .filter(|edge| generated_edge_midpoint_inside_contour(*edge, containing_contour))
        .collect()
}

fn generated_contact_edges_from_overlay_intersection(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(left_shapes) = generated_contour_overlay_shapes(left) else {
        return Vec::new();
    };
    let Some(right_shapes) = generated_contour_overlay_shapes(right) else {
        return Vec::new();
    };
    let Some(intersection) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_shapes,
        &right_shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let mut edges = intersection
        .into_iter()
        .flat_map(|shape| shape.into_iter())
        .flat_map(|contour| {
            let keys = contour
                .into_iter()
                .map(|point| {
                    (
                        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            let mut edges = Vec::new();
            for index in 0..keys.len() {
                let start = keys[index];
                let end = keys[(index + 1) % keys.len()];
                if start != end {
                    edges.push(GeneratedContourEdgeKey::new(start, end));
                }
            }
            edges
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn generated_contact_points_from_contour_intersections(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            if let Some(point) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                left_edge.start,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(left_edge.start);
            }
            if generated_point_key_lies_on_segment(left_edge.end, right_edge.start, right_edge.end)
            {
                points.push(left_edge.end);
            }
            if generated_point_key_lies_on_segment(right_edge.start, left_edge.start, left_edge.end)
            {
                points.push(right_edge.start);
            }
            if generated_point_key_lies_on_segment(right_edge.end, left_edge.start, left_edge.end) {
                points.push(right_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_contour_overlay_shapes(contour: &NodeGeneratedContour) -> Option<NodeOverlayShapes> {
    RoadSurfaceSystem::overlay_union_contours(&[generated_overlay_contour(contour)])
}

fn generated_overlay_contour(contour: &NodeGeneratedContour) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

fn generated_edge_midpoint_inside_contour(
    edge: GeneratedContourEdgeKey,
    contour: &NodeGeneratedContour,
) -> bool {
    let point_x2 = i128::from(edge.start.0) + i128::from(edge.end.0);
    let point_z2 = i128::from(edge.start.1) + i128::from(edge.end.1);
    doubled_point_inside_or_on_generated_contour(point_x2, point_z2, contour)
}

fn doubled_point_inside_or_on_generated_contour(
    point_x2: i128,
    point_z2: i128,
    contour: &NodeGeneratedContour,
) -> bool {
    let keys = generated_contour_keys(contour);
    doubled_point_inside_or_on_generated_keys(point_x2, point_z2, &keys)
}

fn doubled_point_inside_or_on_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> bool {
    if keys.len() < 3 {
        return false;
    }
    let mut inside = false;
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if doubled_point_lies_on_generated_segment(point_x2, point_z2, start, end) {
            return true;
        }
        let start_z2 = i128::from(start.1) * 2;
        let end_z2 = i128::from(end.1) * 2;
        if (start_z2 > point_z2) == (end_z2 > point_z2) {
            continue;
        }
        let start_x2 = i128::from(start.0) * 2;
        let end_x2 = i128::from(end.0) * 2;
        let denominator = end_z2 - start_z2;
        let lhs = (point_x2 - start_x2) * denominator;
        let rhs = (point_z2 - start_z2) * (end_x2 - start_x2);
        let crosses = if denominator > 0 {
            lhs < rhs
        } else {
            lhs > rhs
        };
        if crosses {
            inside = !inside;
        }
    }
    inside
}

fn doubled_point_lies_on_generated_segment(
    point_x2: i128,
    point_z2: i128,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    let start_x2 = i128::from(start.0) * 2;
    let start_z2 = i128::from(start.1) * 2;
    let end_x2 = i128::from(end.0) * 2;
    let end_z2 = i128::from(end.1) * 2;
    let dx = end_x2 - start_x2;
    let dz = end_z2 - start_z2;
    let px = point_x2 - start_x2;
    let pz = point_z2 - start_z2;
    if px * dz - pz * dx != 0 {
        return false;
    }
    point_x2 >= start_x2.min(end_x2)
        && point_x2 <= start_x2.max(end_x2)
        && point_z2 >= start_z2.min(end_z2)
        && point_z2 <= start_z2.max(end_z2)
}

fn node_generated_contact_contours(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    let max_passes = contours.len().saturating_mul(contours.len()).max(1) * 4;
    for _ in 0..max_passes {
        let Some((contour_index, edge, insert_key)) =
            generated_contact_contour_noding_candidate(contours, constraints)
        else {
            return Ok(());
        };
        insert_key_on_generated_contour(contours, constraints, contour_index, edge, insert_key)?;
    }
    Ok(())
}

fn generated_contact_contour_noding_candidate(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Option<(usize, GeneratedContourDirectedEdge, NodeRailPointKey)> {
    for left_index in 0..contours.len() {
        for right_index in left_index + 1..contours.len() {
            let left = &contours[left_index];
            let right = &contours[right_index];
            if !generated_contours_support_contact_noding(left, right) {
                continue;
            }
            if let Some((edge, insert_key)) =
                generated_contact_point_on_edge_noding_candidate(left, right, constraints)
            {
                return Some((left_index, edge, insert_key));
            }
            if let Some((edge, insert_key)) =
                generated_contact_point_on_edge_noding_candidate(right, left, constraints)
            {
                return Some((right_index, edge, insert_key));
            }
            if let Some((left_edge, _right_edge, insert_key)) =
                generated_contact_edge_intersection_noding_candidate(left, right, constraints)
            {
                return Some((left_index, left_edge, insert_key));
            }
        }
    }
    None
}

fn generated_contours_support_contact_noding(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> bool {
    let Some(left_kind) = generated_contour_band_kind(left) else {
        return false;
    };
    let Some(right_kind) = generated_contour_band_kind(right) else {
        return false;
    };
    if generated_contact_constraint_kind(left_kind, right_kind).is_none() {
        return false;
    }
    let Some(left_owner) = left.owner else {
        return false;
    };
    let Some(right_owner) = right.owner else {
        return false;
    };
    left_owner != right_owner
}

fn generated_contact_point_on_edge_noding_candidate(
    edge_contour: &NodeGeneratedContour,
    point_contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
) -> Option<(GeneratedContourDirectedEdge, NodeRailPointKey)> {
    let edge_keys = generated_contour_keys(edge_contour);
    for edge in generated_contour_directed_edges(edge_contour) {
        for point_key in generated_contour_keys(point_contour) {
            if edge_keys.contains(&point_key)
                || !generated_point_key_lies_on_segment(point_key, edge.start, edge.end)
                || !generated_contact_noding_point_has_explicit_roles(
                    edge_contour,
                    point_contour,
                    constraints,
                    point_key,
                )
            {
                continue;
            }
            return Some((edge, point_key));
        }
    }
    None
}

fn generated_contact_edge_intersection_noding_candidate(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
) -> Option<(
    GeneratedContourDirectedEdge,
    GeneratedContourDirectedEdge,
    NodeRailPointKey,
)> {
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            let Some(intersection) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) else {
                continue;
            };
            if !generated_contact_noding_point_has_explicit_roles(
                left,
                right,
                constraints,
                intersection,
            ) {
                continue;
            }
            return Some((left_edge, right_edge, intersection));
        }
    }
    None
}

fn generated_contact_noding_point_has_explicit_roles(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    let Some(left_kind) = generated_contour_band_kind(left) else {
        return false;
    };
    let Some(right_kind) = generated_contour_band_kind(right) else {
        return false;
    };
    let Some(contact_kind) = generated_contact_constraint_kind(left_kind, right_kind) else {
        return false;
    };
    generated_contact_point_has_explicit_roles(
        left_kind,
        right_kind,
        left,
        right,
        constraints,
        point,
        contact_kind,
    )
}

fn generated_contact_constraint_kind(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
) -> Option<NodeRailConstraintKind> {
    if left_kind == right_kind {
        return generated_contour_supports_same_band_contact(left_kind).then_some(
            NodeRailConstraintKind::BandBoundary {
                left_kind,
                right_kind,
            },
        );
    }
    if (is_carriageway(left_kind) && is_curb_or_shoulder(right_kind))
        || (is_curb_or_shoulder(left_kind) && is_carriageway(right_kind))
    {
        return Some(NodeRailConstraintKind::AsphaltCurbContact);
    }
    if (is_curb_or_shoulder(left_kind) && is_sidewalk(right_kind))
        || (is_sidewalk(left_kind) && is_curb_or_shoulder(right_kind))
    {
        return Some(NodeRailConstraintKind::CurbSidewalkContact);
    }
    None
}

fn generated_same_band_point_contact_has_explicit_roles(
    kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> bool {
    generated_contour_supports_same_band_role(kind)
        && generated_same_band_boundary_role_at_contour_vertex(left, constraints, point).is_some()
        && generated_same_band_boundary_role_at_contour_vertex(right, constraints, point).is_some()
}

fn generated_contact_point_has_explicit_roles(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
    contact_kind: NodeRailConstraintKind,
) -> bool {
    if left_kind == right_kind {
        return generated_same_band_point_contact_has_explicit_roles(
            left_kind,
            left,
            right,
            constraints,
            point,
        );
    }
    match contact_kind {
        NodeRailConstraintKind::AsphaltCurbContact => {
            generated_curb_contact_role_at_point(left, right, constraints, point)
                == Some(GeneratedSameBandBoundaryRole::LowerSide)
        }
        NodeRailConstraintKind::CurbSidewalkContact => {
            generated_curb_contact_role_at_point(left, right, constraints, point)
                == Some(GeneratedSameBandBoundaryRole::RaisedSide)
                && generated_sidewalk_contact_role_at_point(left, right, constraints, point)
                    == Some(GeneratedSameBandBoundaryRole::LowerSide)
        }
        _ => true,
    }
}

fn generated_curb_contact_role_at_point(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    [left, right]
        .into_iter()
        .find(|contour| generated_contour_band_kind(contour).is_some_and(is_curb_or_shoulder))
        .and_then(|contour| {
            generated_same_band_boundary_role_at_contour_vertex(contour, constraints, point)
        })
}

fn generated_sidewalk_contact_role_at_point(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    point: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    [left, right]
        .into_iter()
        .find(|contour| generated_contour_band_kind(contour).is_some_and(is_sidewalk))
        .and_then(|contour| {
            generated_same_band_boundary_role_at_contour_vertex(contour, constraints, point)
        })
}

fn generated_same_band_contact_constraint_key(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraintKey> {
    generated_same_band_contact_constraint(constraint).map(GeneratedSameBandContactConstraint::key)
}

fn generated_same_band_contact_constraint(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraint> {
    let Some(kind) = generated_contact_constraint_kind_from_constraint(constraint.kind) else {
        return None;
    };
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if owner == opposite_owner {
        return None;
    }
    let points = constraint.points_xz.as_slice();
    if points.len() != 2 {
        return None;
    }
    Some(GeneratedSameBandContactConstraint {
        kind,
        owner: owner.min(opposite_owner),
        opposite_owner: owner.max(opposite_owner),
        start: road_point_key(points[0]),
        end: road_point_key(points[1]),
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
    })
}

fn generated_contact_constraint_kind_from_constraint(
    kind: NodeRailConstraintKind,
) -> Option<NodeRailConstraintKind> {
    match kind {
        NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::AsphaltCurbContact
        | NodeRailConstraintKind::CurbSidewalkContact => Some(kind),
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => generated_contact_constraint_kind(left_kind, right_kind)
            .is_some()
            .then_some(kind),
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::FootprintSeam { .. } => None,
    }
}

fn insert_key_on_generated_contour(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    contour_index: usize,
    edge: GeneratedContourDirectedEdge,
    insert_key: NodeRailPointKey,
) -> Result<(), NodeRailGenerationError> {
    let Some(contour) = contours.get_mut(contour_index) else {
        return Ok(());
    };
    let mut keys = generated_contour_keys(contour);
    if insert_key_on_generated_contour_edge(&mut keys, edge.start, edge.end, insert_key) {
        set_generated_contour_from_keys(contour, constraints, keys)?;
    }
    Ok(())
}

fn set_generated_contour_from_keys(
    contour: &mut NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
    keys: Vec<NodeRailPointKey>,
) -> Result<(), NodeRailGenerationError> {
    let points = keys
        .into_iter()
        .map(road_point_from_key)
        .collect::<Vec<_>>();
    let polyline = cleaned_closed_contour(
        contour.kind,
        contour.source_mouth_order_index,
        contour.source_band_index,
        points,
    )?;
    contour.points_xz = polyline_to_road_points(&polyline);
    contour.backend_polyline = polyline;
    update_generated_band_contour_constraint(contour, constraints);
    Ok(())
}

fn update_generated_band_contour_constraint(
    contour: &NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
) {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return;
    };
    for constraint in constraints {
        if matches!(
            constraint.kind,
            NodeRailConstraintKind::BandContour { kind: constraint_kind }
                if constraint_kind == kind
        ) && constraint.source_mouth_order_index == contour.source_mouth_order_index
            && constraint.source_band_index == contour.source_band_index
            && constraint.owner == contour.owner
        {
            constraint.points_xz = contour.points_xz.clone();
        }
    }
}

fn shared_generated_contour_edges(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut left_edges = generated_contour_edges(left);
    let mut right_edges = generated_contour_edges(right);
    left_edges.sort_unstable();
    left_edges.dedup();
    right_edges.sort_unstable();
    right_edges.dedup();
    left_edges
        .into_iter()
        .filter(|edge| right_edges.binary_search(edge).is_ok())
        .collect()
}

fn shared_generated_contour_points(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut left_points = generated_contour_keys(left);
    let mut right_points = generated_contour_keys(right);
    left_points.sort_unstable();
    left_points.dedup();
    right_points.sort_unstable();
    right_points.dedup();
    left_points
        .into_iter()
        .filter(|point| right_points.binary_search(point).is_ok())
        .collect()
}

fn generated_contour_edges(contour: &NodeGeneratedContour) -> Vec<GeneratedContourEdgeKey> {
    let keys = generated_contour_keys(contour);
    let mut edges = Vec::new();
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if start != end {
            edges.push(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges
}

fn generated_contour_directed_edges(
    contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourDirectedEdge> {
    let keys = generated_contour_keys(contour);
    let mut edges = Vec::new();
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if start != end {
            edges.push(GeneratedContourDirectedEdge { start, end });
        }
    }
    edges
}

fn generated_contour_keys(contour: &NodeGeneratedContour) -> Vec<NodeRailPointKey> {
    contour
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect()
}

fn generated_same_band_boundary_role_at_contour_vertex(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    key: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return None;
    };
    if !generated_contour_supports_same_band_role(kind) {
        return None;
    }
    let keys = generated_contour_keys(contour);
    if keys.len() < 2 {
        return None;
    }
    let mut roles = Vec::new();
    for index in 0..keys.len() {
        if keys[index] != key {
            continue;
        }
        let previous_key = keys[if index == 0 {
            keys.len() - 1
        } else {
            index - 1
        }];
        let next_key = keys[(index + 1) % keys.len()];
        collect_generated_same_band_role_on_segment(
            contour,
            constraints,
            previous_key,
            key,
            &mut roles,
        );
        collect_generated_same_band_role_on_segment(
            contour,
            constraints,
            key,
            next_key,
            &mut roles,
        );
    }
    roles.sort_unstable();
    roles.dedup();
    if roles.len() == 1 {
        return roles.first().copied();
    }
    generated_same_band_boundary_role_at_key(contour, constraints, key)
}

fn generated_contour_supports_same_band_role(kind: RoadSurfaceBandKind) -> bool {
    matches!(
        kind,
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    )
}

fn generated_contour_supports_same_band_contact(kind: RoadSurfaceBandKind) -> bool {
    matches!(
        kind,
        RoadSurfaceBandKind::Carriageway
            | RoadSurfaceBandKind::CurbOrShoulder
            | RoadSurfaceBandKind::Sidewalk
    )
}

fn collect_generated_same_band_role_on_segment(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    roles: &mut Vec<GeneratedSameBandBoundaryRole>,
) {
    if start == end {
        return;
    }
    let Some(owner) = contour.owner else {
        return;
    };
    let Some(kind) = generated_contour_band_kind(contour) else {
        return;
    };
    for constraint in constraints
        .iter()
        .filter(|constraint| generated_constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| generated_constraint_contains_key_segment(constraint, start, end))
    {
        if let Some(role) = generated_boundary_role_from_constraint_kind(kind, constraint.kind) {
            roles.push(role);
        }
    }
}

fn generated_same_band_boundary_role_at_key(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    key: NodeRailPointKey,
) -> Option<GeneratedSameBandBoundaryRole> {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return None;
    };
    if !generated_contour_supports_same_band_role(kind) {
        return None;
    }
    let Some(owner) = contour.owner else {
        return None;
    };
    let mut has_lower_side = false;
    let mut has_raised_side = false;
    for constraint in constraints
        .iter()
        .filter(|constraint| generated_constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| generated_constraint_touches_key(constraint, key))
    {
        match generated_boundary_role_from_constraint_kind(kind, constraint.kind) {
            Some(GeneratedSameBandBoundaryRole::LowerSide) => has_lower_side = true,
            Some(GeneratedSameBandBoundaryRole::RaisedSide) => has_raised_side = true,
            None => {}
        }
    }
    if has_raised_side {
        Some(GeneratedSameBandBoundaryRole::RaisedSide)
    } else if has_lower_side {
        Some(GeneratedSameBandBoundaryRole::LowerSide)
    } else {
        None
    }
}

fn generated_boundary_role_from_constraint_kind(
    contour_kind: RoadSurfaceBandKind,
    kind: NodeRailConstraintKind,
) -> Option<GeneratedSameBandBoundaryRole> {
    match contour_kind {
        RoadSurfaceBandKind::CurbOrShoulder => match kind {
            NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::AsphaltBoundary { .. } => {
                Some(GeneratedSameBandBoundaryRole::LowerSide)
            }
            NodeRailConstraintKind::CurbSidewalkContact => {
                Some(GeneratedSameBandBoundaryRole::RaisedSide)
            }
            NodeRailConstraintKind::FullRoadbedContour
            | NodeRailConstraintKind::BandContour { .. }
            | NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::BandBoundary { .. } => None,
        },
        RoadSurfaceBandKind::Sidewalk => match kind {
            NodeRailConstraintKind::CurbSidewalkContact => {
                Some(GeneratedSameBandBoundaryRole::LowerSide)
            }
            NodeRailConstraintKind::FootprintSeam {
                adjacent_kind: RoadSurfaceBandKind::Sidewalk,
            } => Some(GeneratedSameBandBoundaryRole::RaisedSide),
            NodeRailConstraintKind::FullRoadbedContour
            | NodeRailConstraintKind::BandContour { .. }
            | NodeRailConstraintKind::SpanHandoff { .. }
            | NodeRailConstraintKind::AsphaltBoundary { .. }
            | NodeRailConstraintKind::AsphaltCurbContact
            | NodeRailConstraintKind::FootprintSeam { .. }
            | NodeRailConstraintKind::BandBoundary { .. } => None,
        },
        _ => None,
    }
}

fn generated_constraint_applies_to_owner(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
) -> bool {
    if constraint.owner.is_some() || constraint.opposite_owner.is_some() {
        return constraint.owner == Some(owner) || constraint.opposite_owner == Some(owner);
    }
    match constraint.kind {
        NodeRailConstraintKind::FullRoadbedContour => true,
        NodeRailConstraintKind::BandContour { kind }
        | NodeRailConstraintKind::SpanHandoff { kind }
        | NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: kind,
        } => kind == owner.kind(),
        NodeRailConstraintKind::AsphaltBoundary { adjacent_kind } => {
            is_carriageway(owner.kind()) || adjacent_kind == owner.kind()
        }
        NodeRailConstraintKind::AsphaltCurbContact => {
            is_carriageway(owner.kind()) || is_curb_or_shoulder(owner.kind())
        }
        NodeRailConstraintKind::CurbSidewalkContact => {
            is_curb_or_shoulder(owner.kind()) || is_sidewalk(owner.kind())
        }
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => left_kind == owner.kind() || right_kind == owner.kind(),
    }
}

fn generated_constraint_contains_key_segment(
    constraint: &NodeRailConstraint,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    let Some(first) = constraint.points_xz.first().copied() else {
        return false;
    };
    let mut previous_key = road_point_key(first);
    for point in constraint.points_xz.iter().copied().skip(1) {
        let next_key = road_point_key(point);
        if generated_point_key_lies_on_segment(start, previous_key, next_key)
            && generated_point_key_lies_on_segment(end, previous_key, next_key)
        {
            return true;
        }
        previous_key = next_key;
    }
    false
}

fn generated_constraint_touches_key(
    constraint: &NodeRailConstraint,
    key: NodeRailPointKey,
) -> bool {
    constraint.points_xz.windows(2).any(|segment| {
        generated_point_key_lies_on_segment(
            key,
            road_point_key(segment[0]),
            road_point_key(segment[1]),
        )
    })
}

fn generated_constraint_directed_edges(
    constraint: &NodeRailConstraint,
) -> Vec<GeneratedContourDirectedEdge> {
    constraint
        .points_xz
        .windows(2)
        .filter_map(|segment| {
            let start = road_point_key(segment[0]);
            let end = road_point_key(segment[1]);
            (start != end).then_some(GeneratedContourDirectedEdge { start, end })
        })
        .collect()
}

fn owners_match_unordered(
    owner: Option<NodeBandOwner>,
    opposite_owner: Option<NodeBandOwner>,
    left: NodeBandOwner,
    right: NodeBandOwner,
) -> bool {
    (owner == Some(left) && opposite_owner == Some(right))
        || (owner == Some(right) && opposite_owner == Some(left))
}

fn generated_point_key_lies_on_segment(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    if point == start || point == end {
        return true;
    }
    if start == end {
        return false;
    }
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    if px * dz - pz * dx != 0 {
        return false;
    }
    let inside_x = if start.0 == end.0 {
        point.0 == start.0
    } else {
        point.0 > start.0.min(end.0) && point.0 < start.0.max(end.0)
    };
    let inside_z = if start.1 == end.1 {
        point.1 == start.1
    } else {
        point.1 > start.1.min(end.1) && point.1 < start.1.max(end.1)
    };
    inside_x && inside_z
}

fn insert_key_on_generated_contour_edge(
    keys: &mut Vec<NodeRailPointKey>,
    start_key: NodeRailPointKey,
    end_key: NodeRailPointKey,
    insert_key: NodeRailPointKey,
) -> bool {
    if keys.len() < 2 {
        return false;
    }
    for index in 0..keys.len() {
        let next = if index + 1 == keys.len() {
            0
        } else {
            index + 1
        };
        if keys[index] == start_key && keys[next] == end_key {
            keys.insert(next, insert_key);
            remove_generated_contour_spikes(keys);
            return true;
        }
    }
    false
}

fn remove_generated_contour_spikes(keys: &mut Vec<NodeRailPointKey>) {
    keys.dedup();
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
                removed = true;
                break;
            }
        }
        if !removed {
            return;
        }
    }
}

fn generated_triangle_double_area(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
) -> i128 {
    let ab_x = i128::from(b.0 - a.0);
    let ab_z = i128::from(b.1 - a.1);
    let ac_x = i128::from(c.0 - a.0);
    let ac_z = i128::from(c.1 - a.1);
    ab_x * ac_z - ab_z * ac_x
}

fn generated_segment_overlap_edge(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
) -> Option<GeneratedContourEdgeKey> {
    if a == b
        || c == d
        || generated_triangle_double_area(a, b, c) != 0
        || generated_triangle_double_area(a, b, d) != 0
    {
        return None;
    }
    let mut points = [a, b, c, d]
        .into_iter()
        .filter(|point| {
            generated_point_key_lies_on_segment(*point, a, b)
                && generated_point_key_lies_on_segment(*point, c, d)
        })
        .collect::<Vec<_>>();
    points.sort_by_key(|point| generated_segment_parameter_key(a, b, *point));
    points.dedup();
    match (points.first().copied(), points.last().copied()) {
        (Some(start), Some(end)) if start != end => Some(GeneratedContourEdgeKey::new(start, end)),
        _ => None,
    }
}

fn generated_segment_touch_points(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    if let Some(point) = quantized_proper_segment_intersection(a, b, c, d) {
        points.push(point);
    }
    for point in [a, b] {
        if generated_point_key_lies_on_segment(point, c, d) {
            points.push(point);
        }
    }
    for point in [c, d] {
        if generated_point_key_lies_on_segment(point, a, b) {
            points.push(point);
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

fn generated_segment_parameter_key(
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    point: NodeRailPointKey,
) -> i128 {
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    let px = i128::from(point.0 - start.0);
    let pz = i128::from(point.1 - start.1);
    px * dx + pz * dz
}

fn quantized_proper_segment_intersection(
    a: NodeRailPointKey,
    b: NodeRailPointKey,
    c: NodeRailPointKey,
    d: NodeRailPointKey,
) -> Option<NodeRailPointKey> {
    if a == b || c == d {
        return None;
    }
    let ab_c = generated_triangle_double_area(a, b, c);
    let ab_d = generated_triangle_double_area(a, b, d);
    let cd_a = generated_triangle_double_area(c, d, a);
    let cd_b = generated_triangle_double_area(c, d, b);
    if ab_c == 0 || ab_d == 0 || cd_a == 0 || cd_b == 0 {
        return None;
    }
    if (ab_c > 0) == (ab_d > 0) || (cd_a > 0) == (cd_b > 0) {
        return None;
    }

    let r_x = i128::from(b.0 - a.0);
    let r_z = i128::from(b.1 - a.1);
    let s_x = i128::from(d.0 - c.0);
    let s_z = i128::from(d.1 - c.1);
    let offset_x = i128::from(c.0 - a.0);
    let offset_z = i128::from(c.1 - a.1);
    let denominator = r_x * s_z - r_z * s_x;
    if denominator == 0 {
        return None;
    }
    let numerator = offset_x * s_z - offset_z * s_x;
    let x_num = i128::from(a.0) * denominator + r_x * numerator;
    let z_num = i128::from(a.1) * denominator + r_z * numerator;
    let intersection = (
        div_round_nearest_i128(x_num, denominator)?,
        div_round_nearest_i128(z_num, denominator)?,
    );
    if intersection == a || intersection == b || intersection == c || intersection == d {
        None
    } else {
        Some(intersection)
    }
}

fn div_round_nearest_i128(numerator: i128, denominator: i128) -> Option<i64> {
    if denominator == 0 {
        return None;
    }
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    i64::try_from(rounded).ok()
}

fn generated_contour_band_kind(contour: &NodeGeneratedContour) -> Option<RoadSurfaceBandKind> {
    match contour.kind {
        NodeGeneratedContourKind::Band { kind } => Some(kind),
        NodeGeneratedContourKind::FullRoadbed => None,
    }
}

fn road_point_key(point: RoadVec2) -> NodeRailPointKey {
    (
        (point.x * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point.y * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
}

fn road_point_from_key(point: NodeRailPointKey) -> RoadVec2 {
    RoadVec2::new(
        point.0 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
        point.1 as f64 / ROAD_OVERLAY_COORDINATE_SCALE,
    )
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
            let band_owners: Vec<NodeBandOwner> = mouth
                .band_intervals
                .iter()
                .map(|interval| {
                    let owner = NodeBandOwner::new(interval.band_kind, next_owner_index);
                    next_owner_index += 1;
                    owner
                })
                .collect();
            let mut terminal_owner_by_source =
                BTreeMap::<(RoadSurfaceBandKind, usize), NodeBandOwner>::new();
            for (interval, owner) in mouth.band_intervals.iter().zip(&band_owners) {
                terminal_owner_by_source.insert((interval.band_kind, interval.band_index), *owner);
            }
            let terminal_end_band_owners = mouth
                .terminal_end_bands
                .iter()
                .map(|end_band| {
                    let key = (end_band.band_kind, end_band.source_band_index);
                    if let Some(owner) = terminal_owner_by_source.get(&key).copied() {
                        owner
                    } else {
                        let owner = NodeBandOwner::new(end_band.band_kind, next_owner_index);
                        next_owner_index += 1;
                        terminal_owner_by_source.insert(key, owner);
                        owner
                    }
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
        assert_eq!(contours.constraints.len(), 18);
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
