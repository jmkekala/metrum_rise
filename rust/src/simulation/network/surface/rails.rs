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
    pub(crate) height_carrier_points_by_source:
        BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeGeneratedContour {
    pub(crate) kind: NodeGeneratedContourKind,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) owner: Option<NodeBandOwner>,
    pub(crate) claim_priority: NodeGeneratedContourClaimPriority,
    pub(crate) points_xz: Vec<RoadVec2>,
    pub(crate) height_points_world: Option<Vec<RoadVec3>>,
    pub(crate) backend_polyline: RoadPolyline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) enum NodeGeneratedContourClaimPriority {
    JoinOrCap,
    SideJoin,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CurbAsphaltContactOppositeOwnerMode {
    RoleOnly,
    EndpointOrAdjacentCarriageway,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedPointContourLocation {
    Outside,
    Boundary,
    Inside,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct GeneratedOwnerContactGroupKey {
    owner: NodeBandOwner,
    kind: RoadSurfaceBandKind,
    claim_priority: NodeGeneratedContourClaimPriority,
    source_mouth_order_index: usize,
    source_band_index: Option<usize>,
}

#[derive(Clone, Debug)]
struct GeneratedOwnerContactGroup {
    key: GeneratedOwnerContactGroupKey,
    contour_indices: Vec<usize>,
    shapes: NodeOverlayShapes,
    shape_edges: Vec<GeneratedContourDirectedEdge>,
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
        let mut height_carrier_points_by_source =
            BTreeMap::<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>::new();

        for (mouth, mouth_owners) in input.mouths.iter().zip(&owners_by_mouth) {
            push_full_roadbed_contour(mouth, &mut contours, &mut constraints)?;

            for (band_index, interval) in mouth.band_intervals.iter().enumerate() {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    interval.band_index,
                    interval.band_kind,
                    interval_height_carrier_points(interval),
                );
                let owner = mouth_owners.band_owners[band_index];
                push_band_contour(mouth, interval, owner, &mut contours, &mut constraints)?;
            }
            for end_band in &mouth.terminal_end_bands {
                push_band_height_carrier_points(
                    &mut height_carrier_points_by_source,
                    mouth.order_index,
                    end_band.source_band_index,
                    end_band.band_kind,
                    end_band.contour_world.iter().copied(),
                );
            }

            push_terminal_end_band_contours(
                input.piece_kind,
                mouth,
                &mouth.terminal_end_bands,
                mouth_owners,
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
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        append_generated_same_band_contact_constraints(
            input.piece_kind,
            &contours,
            &mut constraints,
        );
        node_generated_contact_contours(&mut contours, &mut constraints)?;
        node_generated_contact_constraint_points_on_contours(
            input.piece_kind,
            &mut contours,
            &mut constraints,
        )?;
        canonicalize_generated_contact_constraints_to_source_contours(&contours, &mut constraints);
        Ok(Self {
            node_id: input.node_id,
            piece_kind: input.piece_kind,
            contours,
            constraints,
            height_carrier_points_by_source,
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
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(first.mouth_world));
    push_road_path_point(&mut points, xz(last.mouth_world));
    append_world_path_xz(&mut points, last.path_world.iter().skip(1));
    append_world_path_xz(&mut points, first.path_world.iter().rev());
    remove_closing_road_path_duplicate(&mut points);
    push_generated_contour(
        NodeGeneratedContourKind::FullRoadbed,
        mouth.order_index,
        None,
        None,
        NodeGeneratedContourClaimPriority::Footprint,
        NodeRailConstraintKind::FullRoadbedContour,
        points,
        None,
        contours,
        constraints,
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
    let last_band_index = mouth.band_intervals.len().saturating_sub(1);
    if mouth.uses_sampled_band_domain_paths
        && interval
            .start_path_world
            .len()
            .min(interval.end_path_world.len())
            > 2
    {
        return push_path_band_contour(
            kind,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &interval.start_path_world,
            &interval.end_path_world,
            contours,
            constraints,
        );
    }
    if interval.band_index == 0 && interval.start_path_world.len() > 2 {
        let inner_path = subdivided_world_chord(
            interval.mouth_end_world,
            interval.endpoint_end_world,
            interval.start_path_world.len(),
        );
        return push_path_strip_contours(
            kind,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &interval.start_path_world,
            &inner_path,
            contours,
            constraints,
        );
    }
    if interval.band_index == last_band_index && interval.end_path_world.len() > 2 {
        let inner_path = subdivided_world_chord(
            interval.mouth_start_world,
            interval.endpoint_start_world,
            interval.end_path_world.len(),
        );
        return push_path_strip_contours(
            kind,
            mouth.order_index,
            Some(interval.band_index),
            Some(owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: interval.band_kind,
            },
            &inner_path,
            &interval.end_path_world,
            contours,
            constraints,
        );
    }
    let mut points_world = Vec::new();
    push_world_path_point(&mut points_world, interval.mouth_start_world);
    push_world_path_point(&mut points_world, interval.mouth_end_world);
    if interval.band_index == last_band_index {
        append_world_path_points(&mut points_world, interval.end_path_world.iter().skip(1));
    } else {
        push_world_path_point(&mut points_world, interval.endpoint_end_world);
    }
    if interval.band_index == 0 {
        append_world_path_points(&mut points_world, interval.start_path_world.iter().rev());
    } else {
        push_world_path_point(&mut points_world, interval.endpoint_start_world);
    }
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour(
        kind,
        mouth.order_index,
        Some(interval.band_index),
        Some(owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: interval.band_kind,
        },
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn interval_height_carrier_points(
    interval: &NodeInputBandInterval,
) -> impl Iterator<Item = RoadVec3> + '_ {
    [
        interval.endpoint_start_world,
        interval.endpoint_end_world,
        interval.mouth_end_world,
        interval.mouth_start_world,
    ]
    .into_iter()
    .chain(interval.start_path_world.iter().copied())
    .chain(interval.end_path_world.iter().copied())
}

fn push_band_height_carrier_points(
    points_by_source: &mut BTreeMap<(RoadSurfaceBandKind, usize, usize), Vec<RoadVec2>>,
    mouth_order_index: usize,
    source_band_index: usize,
    kind: RoadSurfaceBandKind,
    points_world: impl IntoIterator<Item = RoadVec3>,
) {
    let points = points_by_source
        .entry((kind, mouth_order_index, source_band_index))
        .or_default();
    for point in points_world {
        push_road_path_point(points, xz(point));
    }
}

fn push_path_strip_contours(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let mut first_error = None;
    let mut pushed = false;
    for points_world in path_strip_contours_world(start_path_world, end_path_world) {
        let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
        match push_generated_contour(
            kind,
            mouth_order_index,
            band_index,
            owner,
            claim_priority,
            constraint_kind,
            points,
            Some(points_world),
            contours,
            constraints,
        ) {
            Ok(()) => pushed = true,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        };
    }
    if pushed {
        Ok(())
    } else {
        Err(
            first_error.unwrap_or(NodeRailGenerationError::DegenerateContour {
                kind,
                mouth_order_index,
                band_index,
                area_m2: 0.0,
                vertex_count: 0,
            }),
        )
    }
}

fn push_generated_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    points: Vec<RoadVec2>,
    height_points_world: Option<Vec<RoadVec3>>,
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let contour = cleaned_closed_contour(kind, mouth_order_index, band_index, points)?;
    let points_xz = polyline_to_road_points(&contour);
    let height_points_world = height_points_world
        .as_deref()
        .and_then(|points_world| align_height_points_to_contour(&points_xz, points_world));
    contours.push(NodeGeneratedContour {
        kind,
        source_mouth_order_index: mouth_order_index,
        source_band_index: band_index,
        owner,
        claim_priority,
        points_xz: points_xz.clone(),
        height_points_world,
        backend_polyline: contour,
    });
    push_constraint(
        constraints,
        constraint_kind,
        mouth_order_index,
        band_index,
        None,
        owner,
        None,
        points_xz,
    )
}

fn push_path_band_contour(
    kind: NodeGeneratedContourKind,
    mouth_order_index: usize,
    band_index: Option<usize>,
    owner: Option<NodeBandOwner>,
    claim_priority: NodeGeneratedContourClaimPriority,
    constraint_kind: NodeRailConstraintKind,
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let mut points_world = Vec::with_capacity(start_path_world.len() + end_path_world.len());
    append_world_path_points(&mut points_world, start_path_world.iter());
    append_world_path_points(&mut points_world, end_path_world.iter().rev());
    remove_closing_world_path_duplicate(&mut points_world);
    let points = points_world.iter().copied().map(xz).collect::<Vec<_>>();
    push_generated_contour(
        kind,
        mouth_order_index,
        band_index,
        owner,
        claim_priority,
        constraint_kind,
        points,
        Some(points_world),
        contours,
        constraints,
    )
}

fn path_strip_contours_world(
    start_path_world: &[RoadVec3],
    end_path_world: &[RoadVec3],
) -> Vec<Vec<RoadVec3>> {
    let point_count = start_path_world.len().min(end_path_world.len());
    if point_count < 2 {
        return Vec::new();
    }
    let mut strips = Vec::with_capacity(point_count - 1);
    for index in 0..point_count - 1 {
        let mut points = Vec::with_capacity(4);
        push_world_path_point(&mut points, start_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index]);
        push_world_path_point(&mut points, end_path_world[index + 1]);
        push_world_path_point(&mut points, start_path_world[index + 1]);
        remove_closing_world_path_duplicate(&mut points);
        strips.push(points);
    }
    strips
}

fn subdivided_world_chord(start: RoadVec3, end: RoadVec3, point_count: usize) -> Vec<RoadVec3> {
    if point_count < 2 {
        return vec![start, end];
    }
    (0..point_count)
        .map(|index| {
            let t = index as f64 / (point_count - 1) as f64;
            start * (1.0 - t) + end * t
        })
        .collect()
}

fn push_terminal_end_band_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    end_bands: &[NodeInputTerminalEndBand],
    mouth_owners: &MouthOwners,
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let owner_by_kind_and_source =
        terminal_owner_by_kind_and_source(mouth, mouth_owners, end_bands, owners);
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
        push_node_side_join_candidate_contours(
            piece_kind,
            mouth,
            end_bands,
            owners,
            contours,
            constraints,
        )?;
        for (end_band, owner) in end_bands.iter().zip(owners) {
            push_terminal_end_band_boundary_constraints(
                piece_kind,
                mouth,
                end_band,
                *owner,
                &owner_by_kind_and_source,
                constraints,
            )?;
        }
        return Ok(());
    }

    let mut groups = BTreeMap::<TerminalEndBandGroupKey, TerminalEndBandGroup>::new();
    for (end_band, owner) in end_bands.iter().zip(owners) {
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
        push_grouped_end_band_candidate_contours(
            mouth,
            key,
            &group.contour_world,
            &group.end_bands,
            NodeGeneratedContourClaimPriority::JoinOrCap,
            contours,
            constraints,
        )?;
        for end_band in group.end_bands {
            push_terminal_end_band_boundary_constraints(
                piece_kind,
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

fn push_node_side_join_candidate_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    end_bands: &[NodeInputTerminalEndBand],
    owners: &[NodeBandOwner],
    contours: &mut Vec<NodeGeneratedContour>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let mut groups = BTreeMap::<TerminalEndBandGroupKey, TerminalEndBandGroup>::new();
    for (end_band, owner) in end_bands.iter().zip(owners) {
        if !node_side_join_end_band_contributes_domain(end_band) {
            continue;
        }
        groups
            .entry(TerminalEndBandGroupKey {
                kind: end_band.band_kind,
                source_band_index: end_band.source_band_index,
                owner: *owner,
                contributes_footprint: node_side_join_end_band_contributes_footprint(
                    piece_kind, end_band,
                ),
            })
            .or_insert_with(|| TerminalEndBandGroup {
                contour_world: Vec::new(),
                end_bands: Vec::new(),
            })
            .push(end_band);
    }

    for (key, group) in groups {
        push_grouped_end_band_candidate_contours(
            mouth,
            key,
            &group.contour_world,
            &group.end_bands,
            NodeGeneratedContourClaimPriority::SideJoin,
            contours,
            constraints,
        )?;
    }

    Ok(())
}

fn terminal_owner_by_kind_and_source(
    mouth: &NodeInputMouth,
    mouth_owners: &MouthOwners,
    end_bands: &[NodeInputTerminalEndBand],
    owners: &[NodeBandOwner],
) -> BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner> {
    let mut owner_by_kind_and_source = BTreeMap::new();
    for (interval, owner) in mouth.band_intervals.iter().zip(&mouth_owners.band_owners) {
        owner_by_kind_and_source.insert((interval.band_kind, interval.band_index), *owner);
    }
    for (end_band, owner) in end_bands.iter().zip(owners) {
        owner_by_kind_and_source.insert((end_band.band_kind, end_band.source_band_index), *owner);
    }
    owner_by_kind_and_source
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
        let mut contour = end_band
            .contour_world
            .iter()
            .map(|point| [point.x, point.z])
            .collect::<Vec<_>>();
        if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
            && RoadSurfaceSystem::overlay_contour_area(&contour) < 0.0
        {
            contour.reverse();
        }
        self.contour_world.push(contour);
        self.end_bands.push(end_band);
    }
}

fn push_grouped_end_band_candidate_contours(
    mouth: &NodeInputMouth,
    key: TerminalEndBandGroupKey,
    contour_world: &[NodeOverlayContour],
    end_bands: &[&NodeInputTerminalEndBand],
    claim_priority: NodeGeneratedContourClaimPriority,
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
                    height_points_world: None,
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
            let source_height_points_world = end_bands
                .iter()
                .flat_map(|end_band| end_band.contour_world.iter().copied())
                .collect::<Vec<_>>();
            let height_points_world =
                align_height_points_to_contour(&points_xz, &source_height_points_world);
            contours.push(NodeGeneratedContour {
                kind,
                source_mouth_order_index: mouth.order_index,
                source_band_index: Some(key.source_band_index),
                owner: Some(key.owner),
                claim_priority,
                points_xz: points_xz.clone(),
                height_points_world,
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
    piece_kind: RoadSurfaceVisualNodePieceKind,
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
        terminal_end_band_material_opposite_owner(mouth, end_band, owner_by_kind_and_source);
    match end_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => {
            if end_band.boundary_mode
                == NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
            {
                push_terminal_curb_asphalt_contact_constraints(
                    piece_kind,
                    mouth,
                    end_band,
                    owner,
                    owner_by_kind_and_source,
                    constraints,
                )?;
                push_terminal_curb_asphalt_side_role_constraints(
                    piece_kind,
                    mouth,
                    end_band,
                    owner,
                    constraints,
                )?;
                if let Some(points) = outer_path.clone() {
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
                return Ok(());
            }
            if end_band.boundary_mode != NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap {
                push_terminal_curb_asphalt_contact_constraints(
                    piece_kind,
                    mouth,
                    end_band,
                    owner,
                    owner_by_kind_and_source,
                    constraints,
                )?;
                push_terminal_curb_asphalt_side_role_constraints(
                    piece_kind,
                    mouth,
                    end_band,
                    owner,
                    constraints,
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

fn push_terminal_curb_asphalt_side_role_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    if curb_asphalt_contact_opposite_owner_mode(piece_kind, end_band)
        != CurbAsphaltContactOppositeOwnerMode::RoleOnly
    {
        return Ok(());
    }
    for (start, end) in terminal_curb_sidewalk_side_edges(end_band) {
        push_terminal_end_band_constraint(
            constraints,
            NodeRailConstraintKind::AsphaltCurbContact,
            mouth.order_index,
            end_band.source_band_index,
            owner,
            None,
            xz(start),
            xz(end),
        )?;
    }
    Ok(())
}

fn terminal_end_band_contributes_footprint(end_band: &NodeInputTerminalEndBand) -> bool {
    !matches!(
        end_band.boundary_mode,
        NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
            | NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
    )
}

fn node_side_join_end_band_contributes_domain(end_band: &NodeInputTerminalEndBand) -> bool {
    matches!(
        end_band.boundary_mode,
        NodeInputTerminalEndBandBoundaryMode::MaterialBand
            | NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand
            | NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
            | NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
            | NodeInputTerminalEndBandBoundaryMode::MaterialBandWithSameOwnerOuterCap
            | NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap
    )
}

fn node_side_join_end_band_contributes_footprint(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    end_band: &NodeInputTerminalEndBand,
) -> bool {
    match end_band.boundary_mode {
        NodeInputTerminalEndBandBoundaryMode::MaterialBand
        | NodeInputTerminalEndBandBoundaryMode::MaterialBandWithSameOwnerOuterCap
        | NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap => true,
        NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
        | NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint => {
            piece_kind == RoadSurfaceVisualNodePieceKind::Bend
                && end_band.band_kind == RoadSurfaceBandKind::CurbOrShoulder
        }
        NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand => false,
    }
}

fn terminal_end_band_material_opposite_owner(
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    if let Some(owner) =
        terminal_generated_material_opposite_owner(end_band, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    match end_band.band_kind {
        RoadSurfaceBandKind::CurbOrShoulder => adjacent_source_band_owner(
            mouth,
            end_band.source_band_index,
            RoadSurfaceBandKind::Sidewalk,
            owner_by_kind_and_source,
        ),
        RoadSurfaceBandKind::Sidewalk => adjacent_source_band_owner(
            mouth,
            end_band.source_band_index,
            RoadSurfaceBandKind::CurbOrShoulder,
            owner_by_kind_and_source,
        ),
        _ => None,
    }
}

fn terminal_generated_material_opposite_owner(
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

fn push_terminal_curb_asphalt_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    mouth: &NodeInputMouth,
    end_band: &NodeInputTerminalEndBand,
    owner: NodeBandOwner,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    constraints: &mut Vec<NodeRailConstraint>,
) -> Result<(), NodeRailGenerationError> {
    let Some(points) = terminal_end_band_inner_contour_path(end_band) else {
        return Ok(());
    };
    let opposite_owner_mode = curb_asphalt_contact_opposite_owner_mode(piece_kind, end_band);
    for segment in points.windows(2) {
        let opposite_owner = terminal_curb_asphalt_opposite_owner_for_inner_segment(
            mouth,
            end_band.source_band_index,
            segment[0],
            segment[1],
            owner_by_kind_and_source,
            opposite_owner_mode,
        );
        push_terminal_end_band_constraint(
            constraints,
            NodeRailConstraintKind::AsphaltCurbContact,
            mouth.order_index,
            end_band.source_band_index,
            owner,
            opposite_owner,
            segment[0],
            segment[1],
        )?;
    }
    Ok(())
}

fn curb_asphalt_contact_opposite_owner_mode(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    end_band: &NodeInputTerminalEndBand,
) -> CurbAsphaltContactOppositeOwnerMode {
    if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal
        && matches!(
            end_band.boundary_mode,
            NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
                | NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
        )
    {
        CurbAsphaltContactOppositeOwnerMode::RoleOnly
    } else {
        CurbAsphaltContactOppositeOwnerMode::EndpointOrAdjacentCarriageway
    }
}

fn terminal_curb_asphalt_opposite_owner_for_inner_segment(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
    mode: CurbAsphaltContactOppositeOwnerMode,
) -> Option<NodeBandOwner> {
    if mode == CurbAsphaltContactOppositeOwnerMode::RoleOnly {
        return None;
    }
    if let Some(owner) =
        terminal_curb_asphalt_endpoint_opposite_owner(mouth, start, end, owner_by_kind_and_source)
    {
        return Some(owner);
    }
    adjacent_source_band_owner(
        mouth,
        source_band_index,
        RoadSurfaceBandKind::Carriageway,
        owner_by_kind_and_source,
    )
}

fn terminal_curb_asphalt_endpoint_opposite_owner(
    mouth: &NodeInputMouth,
    start: RoadVec2,
    end: RoadVec2,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let start_boundary = endpoint_boundary_index_for_point(mouth, start)?;
    let end_boundary = endpoint_boundary_index_for_point(mouth, end)?;
    let (lower_boundary, upper_boundary) = if start_boundary <= end_boundary {
        (start_boundary, end_boundary)
    } else {
        (end_boundary, start_boundary)
    };
    if lower_boundary + 1 != upper_boundary {
        return None;
    }
    let interval = mouth.band_intervals.get(lower_boundary)?;
    if !is_carriageway(interval.band_kind) {
        return None;
    }
    owner_by_kind_and_source
        .get(&(RoadSurfaceBandKind::Carriageway, interval.band_index))
        .copied()
}

fn adjacent_source_band_owner(
    mouth: &NodeInputMouth,
    source_band_index: usize,
    adjacent_kind: RoadSurfaceBandKind,
    owner_by_kind_and_source: &BTreeMap<(RoadSurfaceBandKind, usize), NodeBandOwner>,
) -> Option<NodeBandOwner> {
    let source = mouth.band_intervals.get(source_band_index)?;
    let mut adjacent_owner = None;
    for adjacent_index in [
        source_band_index.checked_sub(1),
        source_band_index.checked_add(1),
    ]
    .into_iter()
    .flatten()
    {
        let Some(adjacent) = mouth.band_intervals.get(adjacent_index) else {
            continue;
        };
        if adjacent.band_kind != adjacent_kind {
            continue;
        }
        let owner = owner_by_kind_and_source
            .get(&(adjacent.band_kind, adjacent.band_index))
            .copied()?;
        if adjacent_owner.replace(owner).is_some() {
            return None;
        }
    }
    (source.band_kind != adjacent_kind)
        .then_some(adjacent_owner)
        .flatten()
}

fn endpoint_boundary_index_for_point(mouth: &NodeInputMouth, point: RoadVec2) -> Option<usize> {
    let key = road_point_key(point);
    mouth
        .boundary_rails
        .iter()
        .find(|rail| road_point_key(xz(rail.endpoint_world)) == key)
        .map(|rail| rail.boundary_index)
}

fn terminal_end_band_has_material_boundary(end_band: &NodeInputTerminalEndBand) -> bool {
    matches!(
        end_band.boundary_mode,
        NodeInputTerminalEndBandBoundaryMode::MaterialBand
            | NodeInputTerminalEndBandBoundaryMode::TerminalMaterialBand
            | NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint
            | NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint
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
        push_road_path_point(&mut cleaned, point);
    }
    if cleaned
        .windows(2)
        .any(|segment| road_point_key(segment[0]) != road_point_key(segment[1]))
    {
        let raw = road_points_to_polyline(cleaned, false);
        let rail = RoadPolyline::create_from_remove_repeat(&raw, RAIL_CONTOUR_POINT_EQUAL_EPS_M);
        (rail.vertex_count() >= 2 && rail.path_length() > RAIL_CONTOUR_POINT_EQUAL_EPS_M)
            .then(|| polyline_to_road_points(&rail))
    } else {
        None
    }
}

fn open_world_path_xz(
    path_world: &[RoadVec3],
    mouth_world: RoadVec3,
    endpoint_world: RoadVec3,
) -> Vec<RoadVec2> {
    let mut points = Vec::new();
    push_road_path_point(&mut points, xz(mouth_world));
    append_world_path_xz(&mut points, path_world.iter());
    push_road_path_point(&mut points, xz(endpoint_world));
    points
}

fn append_world_path_xz<'a>(
    points: &mut Vec<RoadVec2>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_road_path_point(points, xz(*point));
    }
}

fn append_world_path_points<'a>(
    points: &mut Vec<RoadVec3>,
    path_world: impl IntoIterator<Item = &'a RoadVec3>,
) {
    for point in path_world {
        push_world_path_point(points, *point);
    }
}

fn push_road_path_point(points: &mut Vec<RoadVec2>, point: RoadVec2) {
    if points
        .last()
        .is_none_or(|last| road_point_key(*last) != road_point_key(point))
    {
        points.push(point);
    }
}

fn push_world_path_point(points: &mut Vec<RoadVec3>, point: RoadVec3) {
    if points
        .last()
        .is_none_or(|last| road_point_key(xz(*last)) != road_point_key(xz(point)))
    {
        points.push(point);
    }
}

fn remove_closing_road_path_duplicate(points: &mut Vec<RoadVec2>) {
    if points.len() > 1
        && road_point_key(points[0]) == road_point_key(*points.last().expect("len checked"))
    {
        points.pop();
    }
}

fn remove_closing_world_path_duplicate(points: &mut Vec<RoadVec3>) {
    if points.len() > 1
        && road_point_key(xz(points[0])) == road_point_key(xz(*points.last().expect("len checked")))
    {
        points.pop();
    }
}

fn align_height_points_to_contour(
    contour_points_xz: &[RoadVec2],
    source_points_world: &[RoadVec3],
) -> Option<Vec<RoadVec3>> {
    let mut height_by_key = BTreeMap::<NodeRailPointKey, f64>::new();
    for point in source_points_world {
        let key = road_point_key(xz(*point));
        if let Some(existing_height_m) = height_by_key.get(&key)
            && (*existing_height_m - point.y).abs() > f64::EPSILON
        {
            return None;
        }
        height_by_key.insert(key, point.y);
    }
    contour_points_xz
        .iter()
        .copied()
        .map(|point_xz| {
            height_by_key
                .get(&road_point_key(point_xz))
                .copied()
                .map(|height_m| RoadVec3::new(point_xz.x, height_m, point_xz.y))
        })
        .collect()
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
        open_world_path_xz(&rail.path_world, rail.mouth_world, rail.endpoint_world),
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
    piece_kind: RoadSurfaceVisualNodePieceKind,
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
    append_generated_owner_group_contact_constraints(
        piece_kind,
        contours,
        constraints,
        &mut contact_edges,
    );

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

fn append_generated_owner_group_contact_constraints(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    contact_edges: &mut BTreeSet<GeneratedSameBandContactConstraint>,
) {
    let groups = generated_owner_contact_groups(contours);
    for left_index in 0..groups.len() {
        for right_index in left_index + 1..groups.len() {
            let left = &groups[left_index];
            let right = &groups[right_index];
            let Some(contact_kind) =
                generated_contact_constraint_kind(left.key.kind, right.key.kind)
            else {
                continue;
            };
            if left.key.owner == right.key.owner {
                continue;
            }
            let (owner, opposite_owner) = if left.key.owner <= right.key.owner {
                (left.key.owner, right.key.owner)
            } else {
                (right.key.owner, left.key.owner)
            };
            for edge in generated_owner_group_contact_edges_inside_group(left, right, contours) {
                let has_roles = generated_owner_group_contact_edge_has_explicit_roles(
                    left,
                    right,
                    left,
                    contours,
                    constraints,
                    edge,
                    contact_kind,
                );
                if has_roles {
                    insert_generated_contact_constraint_from_group(
                        contact_edges,
                        contact_kind,
                        owner,
                        opposite_owner,
                        edge,
                        left,
                    );
                }
            }
            for edge in generated_owner_group_contact_edges_inside_group(right, left, contours) {
                let has_roles = generated_owner_group_contact_edge_has_explicit_roles(
                    left,
                    right,
                    right,
                    contours,
                    constraints,
                    edge,
                    contact_kind,
                );
                if has_roles {
                    insert_generated_contact_constraint_from_group(
                        contact_edges,
                        contact_kind,
                        owner,
                        opposite_owner,
                        edge,
                        right,
                    );
                }
            }
            if piece_kind != RoadSurfaceVisualNodePieceKind::Terminal {
                let intersection_source_index = generated_owner_group_intersection_source_index(
                    left_index,
                    right_index,
                    left,
                    right,
                );
                let intersection_source = &groups[intersection_source_index];
                for edge in
                    generated_owner_group_contact_edges_from_overlay_intersection(left, right)
                {
                    let has_roles = generated_owner_group_contact_edge_has_explicit_roles(
                        left,
                        right,
                        intersection_source,
                        contours,
                        constraints,
                        edge,
                        contact_kind,
                    );
                    if has_roles {
                        insert_generated_contact_constraint_from_group(
                            contact_edges,
                            contact_kind,
                            owner,
                            opposite_owner,
                            edge,
                            intersection_source,
                        );
                    }
                }
            }
        }
    }
}

fn generated_owner_group_intersection_source_index(
    left_index: usize,
    right_index: usize,
    left: &GeneratedOwnerContactGroup,
    right: &GeneratedOwnerContactGroup,
) -> usize {
    if is_carriageway(left.key.kind) {
        left_index
    } else if is_carriageway(right.key.kind) {
        right_index
    } else if left.key <= right.key {
        left_index
    } else {
        right_index
    }
}

fn generated_canonical_point_by_projected_key(
    points: &[NodeRailPointKey],
) -> BTreeMap<NodeRailPointKey, NodeRailPointKey> {
    let mut canonical_by_projected_key =
        BTreeMap::<NodeRailPointKey, Option<NodeRailPointKey>>::new();
    for point in points.iter().copied() {
        canonical_by_projected_key
            .entry(generated_project_point_key(point))
            .and_modify(|existing| {
                if *existing != Some(point) {
                    *existing = None;
                }
            })
            .or_insert(Some(point));
    }
    canonical_by_projected_key
        .into_iter()
        .filter_map(|(projected_key, canonical)| {
            canonical.map(|canonical| (projected_key, canonical))
        })
        .collect()
}

fn generated_canonical_point_by_overlay_neighbor_key(
    points: &[NodeRailPointKey],
) -> BTreeMap<NodeRailPointKey, NodeRailPointKey> {
    let mut canonical_by_neighbor_key =
        BTreeMap::<NodeRailPointKey, Option<NodeRailPointKey>>::new();
    for point in points.iter().copied() {
        for neighbor in generated_overlay_neighbor_points(point) {
            canonical_by_neighbor_key
                .entry(neighbor)
                .and_modify(|existing| {
                    if *existing != Some(point) {
                        *existing = None;
                    }
                })
                .or_insert(Some(point));
        }
    }
    canonical_by_neighbor_key
        .into_iter()
        .filter_map(|(neighbor, canonical)| canonical.map(|canonical| (neighbor, canonical)))
        .collect()
}

fn generated_overlay_neighbor_points(point: NodeRailPointKey) -> Vec<NodeRailPointKey> {
    let (x, z) = point;
    let mut points = Vec::with_capacity(25);
    for dx in -2..=2 {
        for dz in -2..=2 {
            points.push((x + dx, z + dz));
        }
    }
    points
}

fn generated_project_point_key(point: NodeRailPointKey) -> NodeRailPointKey {
    (
        generated_coordinate_key_to_mm(point.0),
        generated_coordinate_key_to_mm(point.1),
    )
}

fn generated_coordinate_key_to_mm(value: i64) -> i64 {
    let units_per_mm = (ROAD_OVERLAY_COORDINATE_SCALE / 1000.0) as i64;
    if value >= 0 {
        (value + units_per_mm / 2) / units_per_mm
    } else {
        (value - units_per_mm / 2) / units_per_mm
    }
}

fn generated_owner_contact_groups(
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedOwnerContactGroup> {
    let mut contour_indices_by_key = BTreeMap::<GeneratedOwnerContactGroupKey, Vec<usize>>::new();
    for (contour_index, contour) in contours.iter().enumerate() {
        let Some(owner) = contour.owner else {
            continue;
        };
        let Some(kind) = generated_contour_band_kind(contour) else {
            continue;
        };
        contour_indices_by_key
            .entry(GeneratedOwnerContactGroupKey {
                owner,
                kind,
                claim_priority: contour.claim_priority,
                source_mouth_order_index: contour.source_mouth_order_index,
                source_band_index: contour.source_band_index,
            })
            .or_default()
            .push(contour_index);
    }

    contour_indices_by_key
        .into_iter()
        .filter_map(|(key, contour_indices)| {
            let overlay_contours = contour_indices
                .iter()
                .map(|index| generated_overlay_contour(&contours[*index]))
                .collect::<Vec<_>>();
            let shapes = RoadSurfaceSystem::overlay_union_contours(&overlay_contours)?;
            Some(GeneratedOwnerContactGroup {
                key,
                contour_indices,
                shape_edges: generated_overlay_shapes_directed_edges(&shapes),
                shapes,
            })
        })
        .collect()
}

fn generated_owner_group_contact_edges_inside_group(
    edge_group: &GeneratedOwnerContactGroup,
    containing_group: &GeneratedOwnerContactGroup,
    contours: &[NodeGeneratedContour],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for contour_index in &edge_group.contour_indices {
        let Some(contour) = contours.get(*contour_index) else {
            continue;
        };
        for edge in generated_contour_directed_edges(contour) {
            edges.extend(generated_directed_edge_segments_inside_shape_edges(
                edge,
                &containing_group.shape_edges,
                &containing_group.shapes,
            ));
        }
    }
    edges.into_iter().collect()
}

fn generated_owner_group_contact_edges_from_overlay_intersection(
    left: &GeneratedOwnerContactGroup,
    right: &GeneratedOwnerContactGroup,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(intersection) = RoadSurfaceSystem::overlay_binary_shapes(
        &left.shapes,
        &right.shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let mut edges = intersection
        .into_iter()
        .flat_map(|shape| shape.into_iter())
        .flat_map(|contour| {
            let keys = generated_overlay_contour_keys(&contour);
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

fn generated_owner_group_contact_edge_has_explicit_roles(
    left: &GeneratedOwnerContactGroup,
    right: &GeneratedOwnerContactGroup,
    edge_source: &GeneratedOwnerContactGroup,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
    contact_kind: NodeRailConstraintKind,
) -> bool {
    match contact_kind {
        NodeRailConstraintKind::AsphaltCurbContact => {
            generated_owner_group_contact_role_on_edge(
                left,
                right,
                contours,
                constraints,
                edge,
                is_curb_or_shoulder,
            ) == Some(GeneratedSameBandBoundaryRole::LowerSide)
                || is_carriageway(edge_source.key.kind)
                || generated_owner_group_carriageway_contact_on_edge(
                    left,
                    right,
                    contours,
                    constraints,
                    edge,
                )
        }
        NodeRailConstraintKind::CurbSidewalkContact => {
            let Some(curb_role) = generated_owner_group_contact_role_on_edge(
                left,
                right,
                contours,
                constraints,
                edge,
                is_curb_or_shoulder,
            ) else {
                return false;
            };
            let Some(sidewalk_role) = generated_owner_group_contact_role_on_edge(
                left,
                right,
                contours,
                constraints,
                edge,
                is_sidewalk,
            ) else {
                return false;
            };
            curb_role == GeneratedSameBandBoundaryRole::RaisedSide
                && sidewalk_role == GeneratedSameBandBoundaryRole::LowerSide
        }
        _ => true,
    }
}

fn generated_owner_group_contact_role_on_edge(
    left: &GeneratedOwnerContactGroup,
    right: &GeneratedOwnerContactGroup,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
    predicate: impl Fn(RoadSurfaceBandKind) -> bool,
) -> Option<GeneratedSameBandBoundaryRole> {
    let mut roles = Vec::new();
    for group in [left, right] {
        if !predicate(group.key.kind) {
            continue;
        }
        for contour_index in &group.contour_indices {
            let Some(contour) = contours.get(*contour_index) else {
                continue;
            };
            if let Some(role) = generated_contact_role_on_edge(contour, constraints, edge) {
                roles.push(role);
            }
        }
    }
    roles.sort_unstable();
    roles.dedup();
    if roles.len() == 1 {
        roles.first().copied()
    } else {
        None
    }
}

fn generated_owner_group_carriageway_contact_on_edge(
    left: &GeneratedOwnerContactGroup,
    right: &GeneratedOwnerContactGroup,
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> bool {
    for group in [left, right] {
        if !is_carriageway(group.key.kind) {
            continue;
        }
        for contour_index in &group.contour_indices {
            let Some(contour) = contours.get(*contour_index) else {
                continue;
            };
            if generated_carriageway_contact_on_edge(contour, constraints, edge) {
                return true;
            }
        }
    }
    false
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

fn insert_generated_contact_constraint_from_group(
    contact_edges: &mut BTreeSet<GeneratedSameBandContactConstraint>,
    kind: NodeRailConstraintKind,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    edge: GeneratedContourEdgeKey,
    source_group: &GeneratedOwnerContactGroup,
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
            source_mouth_order_index: source_group.key.source_mouth_order_index,
            source_band_index: source_group.key.source_band_index,
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
                || generated_asphalt_curb_contact_on_carriageway_edge(
                    left,
                    right,
                    constraints,
                    edge,
                )
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

fn generated_asphalt_curb_contact_on_carriageway_edge(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> bool {
    [left, right]
        .into_iter()
        .find(|contour| generated_contour_band_kind(contour).is_some_and(is_carriageway))
        .is_some_and(|contour| generated_carriageway_contact_on_edge(contour, constraints, edge))
}

fn generated_carriageway_contact_on_edge(
    contour: &NodeGeneratedContour,
    constraints: &[NodeRailConstraint],
    edge: GeneratedContourEdgeKey,
) -> bool {
    let Some(owner) = contour.owner else {
        return false;
    };
    constraints
        .iter()
        .filter(|constraint| generated_constraint_applies_to_owner(constraint, owner))
        .filter(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::AsphaltCurbContact
                    | NodeRailConstraintKind::AsphaltBoundary { .. }
                    | NodeRailConstraintKind::BandContour {
                        kind: RoadSurfaceBandKind::Carriageway,
                    }
            )
        })
        .any(|constraint| {
            generated_constraint_contains_key_segment(constraint, edge.start, edge.end)
        })
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
    let mut edges = BTreeSet::new();
    for edge in generated_contour_directed_edges(edge_contour) {
        edges.extend(generated_role_edge_segments_inside_contour(
            edge,
            containing_contour,
        ));
    }
    edges.into_iter().collect()
}

fn generated_directed_edge_segments_inside_shape_edges(
    edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
    containing_shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![edge.start, edge.end];
    for shape_edge in shape_edges {
        if let Some(point) = quantized_proper_segment_intersection(
            edge.start,
            edge.end,
            shape_edge.start,
            shape_edge.end,
        ) {
            keys.push(point);
        }
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, edge.start, edge.end) {
                keys.push(point);
            }
        }
        for point in [edge.start, edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| generated_segment_parameter_key(edge.start, edge.end, *point));
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
        if doubled_point_inside_or_on_overlay_shapes(point_x2, point_z2, containing_shapes) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
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

fn generated_overlay_shapes_directed_edges(
    shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = Vec::new();
    for contour in shapes.iter().flat_map(|shape| shape.iter()) {
        let keys = generated_overlay_contour_keys(contour);
        for index in 0..keys.len() {
            let start = keys[index];
            let end = keys[(index + 1) % keys.len()];
            if start != end {
                edges.push(GeneratedContourDirectedEdge { start, end });
            }
        }
    }
    edges
}

fn generated_overlay_contour_keys(contour: &NodeOverlayContour) -> Vec<NodeRailPointKey> {
    contour
        .iter()
        .copied()
        .map(generated_overlay_point_key)
        .collect()
}

fn generated_overlay_point_key(point: [f64; 2]) -> NodeRailPointKey {
    (
        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
    )
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
    doubled_point_location_in_generated_keys(point_x2, point_z2, keys)
        != GeneratedPointContourLocation::Outside
}

fn doubled_point_inside_or_on_overlay_shapes(
    point_x2: i128,
    point_z2: i128,
    shapes: &NodeOverlayShapes,
) -> bool {
    shapes.iter().any(|shape| {
        let Some(outer) = shape.first() else {
            return false;
        };
        let outer_keys = generated_overlay_contour_keys(outer);
        match doubled_point_location_in_generated_keys(point_x2, point_z2, &outer_keys) {
            GeneratedPointContourLocation::Outside => false,
            GeneratedPointContourLocation::Boundary => true,
            GeneratedPointContourLocation::Inside => shape.iter().skip(1).all(|hole| {
                let hole_keys = generated_overlay_contour_keys(hole);
                doubled_point_location_in_generated_keys(point_x2, point_z2, &hole_keys)
                    != GeneratedPointContourLocation::Inside
            }),
        }
    })
}

fn doubled_point_location_in_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> GeneratedPointContourLocation {
    if keys.len() < 3 {
        return GeneratedPointContourLocation::Outside;
    }
    let mut inside = false;
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if doubled_point_lies_on_generated_segment(point_x2, point_z2, start, end) {
            return GeneratedPointContourLocation::Boundary;
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
    if inside {
        GeneratedPointContourLocation::Inside
    } else {
        GeneratedPointContourLocation::Outside
    }
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

fn node_generated_contact_constraint_points_on_contours(
    piece_kind: RoadSurfaceVisualNodePieceKind,
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) -> Result<(), NodeRailGenerationError> {
    if piece_kind == RoadSurfaceVisualNodePieceKind::Terminal {
        return Ok(());
    }
    let mut candidates =
        generated_contact_constraint_point_noding_candidates(contours, constraints);
    candidates.sort_unstable();
    candidates.dedup();
    for (contour_index, insert_key) in candidates {
        insert_key_on_generated_contour_source_edge(
            contours,
            constraints,
            contour_index,
            insert_key,
        )?;
    }
    Ok(())
}

fn canonicalize_generated_contact_constraints_to_source_contours(
    contours: &[NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
) {
    let mut keys_by_source =
        BTreeMap::<(NodeBandOwner, usize, usize), Vec<NodeRailPointKey>>::new();
    for contour in contours {
        let (Some(owner), Some(source_band_index)) = (contour.owner, contour.source_band_index)
        else {
            continue;
        };
        keys_by_source
            .entry((owner, contour.source_mouth_order_index, source_band_index))
            .or_default()
            .extend(generated_contour_keys(contour));
    }
    let canonical_by_source = keys_by_source
        .into_iter()
        .map(|(source, keys)| {
            (
                source,
                GeneratedSourceContourCanonicalPoints::from_keys(keys),
            )
        })
        .collect::<BTreeMap<_, _>>();

    for constraint in constraints {
        if generated_contact_constraint_kind_from_constraint(constraint.kind).is_none() {
            continue;
        }
        let Some(source_band_index) = constraint.source_band_index else {
            continue;
        };
        let owners = [constraint.owner, constraint.opposite_owner];
        for point in &mut constraint.points_xz {
            let key = road_point_key(*point);
            let mut candidates = owners
                .into_iter()
                .flatten()
                .filter_map(|owner| {
                    canonical_by_source
                        .get(&(
                            owner,
                            constraint.source_mouth_order_index,
                            source_band_index,
                        ))
                        .map(|canonical| canonical.canonicalize(key))
                })
                .filter(|candidate| *candidate != key)
                .collect::<Vec<_>>();
            candidates.sort_unstable();
            candidates.dedup();
            if let [candidate] = candidates.as_slice() {
                *point = road_point_from_key(*candidate);
            }
        }
    }
}

struct GeneratedSourceContourCanonicalPoints {
    keys: BTreeSet<NodeRailPointKey>,
    by_projected_key: BTreeMap<NodeRailPointKey, NodeRailPointKey>,
    by_overlay_neighbor_key: BTreeMap<NodeRailPointKey, NodeRailPointKey>,
}

impl GeneratedSourceContourCanonicalPoints {
    fn from_keys(keys: Vec<NodeRailPointKey>) -> Self {
        Self {
            keys: keys.iter().copied().collect(),
            by_projected_key: generated_canonical_point_by_projected_key(&keys),
            by_overlay_neighbor_key: generated_canonical_point_by_overlay_neighbor_key(&keys),
        }
    }

    fn canonicalize(&self, point: NodeRailPointKey) -> NodeRailPointKey {
        if self.keys.contains(&point) {
            return point;
        }
        self.by_overlay_neighbor_key
            .get(&point)
            .copied()
            .or_else(|| {
                self.by_projected_key
                    .get(&generated_project_point_key(point))
                    .copied()
            })
            .unwrap_or(point)
    }
}

fn generated_contact_constraint_point_noding_candidates(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
) -> Vec<(usize, NodeRailPointKey)> {
    let mut candidates = Vec::new();
    for constraint in constraints {
        if generated_contact_constraint_kind_from_constraint(constraint.kind).is_none() {
            continue;
        }
        let Some(source_band_index) = constraint.source_band_index else {
            continue;
        };
        for (contour_index, contour) in contours.iter().enumerate() {
            let Some(contour_owner) = contour.owner else {
                continue;
            };
            if ![constraint.owner, constraint.opposite_owner].contains(&Some(contour_owner))
                || contour.source_mouth_order_index != constraint.source_mouth_order_index
                || contour.source_band_index != Some(source_band_index)
                || generated_contour_band_kind(contour) != Some(contour_owner.kind())
            {
                continue;
            }
            let contour_keys = generated_contour_keys(contour);
            for point in constraint.points_xz.iter().copied().map(road_point_key) {
                if !contour_keys.contains(&point) {
                    candidates.push((contour_index, point));
                }
            }
        }
    }
    candidates
}

fn insert_key_on_generated_contour_source_edge(
    contours: &mut [NodeGeneratedContour],
    constraints: &mut [NodeRailConstraint],
    contour_index: usize,
    insert_key: NodeRailPointKey,
) -> Result<(), NodeRailGenerationError> {
    let Some(contour) = contours.get_mut(contour_index) else {
        return Ok(());
    };
    let Some(edge) = generated_contour_source_edge_for_key(contour, insert_key) else {
        return Ok(());
    };
    let mut keys = generated_contour_keys(contour);
    if insert_key_on_generated_contour_edge(&mut keys, edge.start, edge.end, insert_key) {
        insert_height_key_on_generated_contour_edge(contour, edge, insert_key);
        set_generated_contour_from_keys(contour, constraints, keys)?;
    }
    Ok(())
}

fn generated_contour_source_edge_for_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> Option<GeneratedContourDirectedEdge> {
    if generated_contour_keys(contour).contains(&point) {
        return None;
    }
    let candidates = generated_contour_directed_edges(contour)
        .into_iter()
        .filter(|edge| {
            generated_point_key_lies_on_segment(point, edge.start, edge.end)
                || generated_point_key_quantization_cell_intersects_segment(
                    point, edge.start, edge.end,
                )
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [edge] => Some(*edge),
        _ => None,
    }
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
        insert_height_key_on_generated_contour_edge(contour, edge, insert_key);
        set_generated_contour_from_keys(contour, constraints, keys)?;
    }
    Ok(())
}

fn insert_height_key_on_generated_contour_edge(
    contour: &mut NodeGeneratedContour,
    edge: GeneratedContourDirectedEdge,
    insert_key: NodeRailPointKey,
) {
    let keys = generated_contour_keys(contour);
    let Some(height_points_world) = contour.height_points_world.as_mut() else {
        return;
    };
    if keys.len() != height_points_world.len() || keys.len() < 2 {
        contour.height_points_world = None;
        return;
    }
    if keys.contains(&insert_key) {
        return;
    }
    for index in 0..keys.len() {
        let next = if index + 1 == keys.len() {
            0
        } else {
            index + 1
        };
        if keys[index] != edge.start || keys[next] != edge.end {
            continue;
        }
        let start_height_m = height_points_world[index].y;
        let end_height_m = height_points_world[next].y;
        let Some(height_m) = height_for_key_on_generated_edge(
            insert_key,
            edge.start,
            edge.end,
            start_height_m,
            end_height_m,
        ) else {
            contour.height_points_world = None;
            return;
        };
        let point = road_point_from_key(insert_key);
        height_points_world.insert(next, RoadVec3::new(point.x, height_m, point.y));
        return;
    }
    contour.height_points_world = None;
}

fn height_for_key_on_generated_edge(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
    start_height_m: f64,
    end_height_m: f64,
) -> Option<f64> {
    if start == end {
        return None;
    }
    let dx = end.0 - start.0;
    let dz = end.1 - start.1;
    let denominator = if dx.abs() >= dz.abs() { dx } else { dz };
    if denominator == 0 {
        return None;
    }
    let numerator = if dx.abs() >= dz.abs() {
        point.0 - start.0
    } else {
        point.1 - start.1
    };
    let t = numerator as f64 / denominator as f64;
    Some(start_height_m + (end_height_m - start_height_m) * t)
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
    if let Some(height_points_world) = contour.height_points_world.as_deref() {
        contour.height_points_world =
            align_height_points_to_contour(&contour.points_xz, height_points_world);
    }
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

fn generated_point_key_quantization_cell_intersects_segment(
    point: NodeRailPointKey,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    if start == end {
        return false;
    }
    let min_x2 = i128::from(point.0) * 2 - 1;
    let max_x2 = i128::from(point.0) * 2 + 1;
    let min_z2 = i128::from(point.1) * 2 - 1;
    let max_z2 = i128::from(point.1) * 2 + 1;
    let segment_start = (i128::from(start.0) * 2, i128::from(start.1) * 2);
    let segment_end = (i128::from(end.0) * 2, i128::from(end.1) * 2);
    if doubled_point_inside_axis_aligned_box(segment_start, min_x2, max_x2, min_z2, max_z2)
        || doubled_point_inside_axis_aligned_box(segment_end, min_x2, max_x2, min_z2, max_z2)
    {
        return true;
    }
    let lower_left = (min_x2, min_z2);
    let lower_right = (max_x2, min_z2);
    let upper_right = (max_x2, max_z2);
    let upper_left = (min_x2, max_z2);
    [
        (lower_left, lower_right),
        (lower_right, upper_right),
        (upper_right, upper_left),
        (upper_left, lower_left),
    ]
    .into_iter()
    .any(|(edge_start, edge_end)| {
        doubled_segments_intersect(segment_start, segment_end, edge_start, edge_end)
    })
}

fn doubled_point_inside_axis_aligned_box(
    point: (i128, i128),
    min_x: i128,
    max_x: i128,
    min_z: i128,
    max_z: i128,
) -> bool {
    point.0 >= min_x && point.0 <= max_x && point.1 >= min_z && point.1 <= max_z
}

fn doubled_segments_intersect(
    a: (i128, i128),
    b: (i128, i128),
    c: (i128, i128),
    d: (i128, i128),
) -> bool {
    let ab_c = doubled_triangle_area2(a, b, c);
    let ab_d = doubled_triangle_area2(a, b, d);
    let cd_a = doubled_triangle_area2(c, d, a);
    let cd_b = doubled_triangle_area2(c, d, b);
    if ab_c == 0 && doubled_point_on_segment(c, a, b) {
        return true;
    }
    if ab_d == 0 && doubled_point_on_segment(d, a, b) {
        return true;
    }
    if cd_a == 0 && doubled_point_on_segment(a, c, d) {
        return true;
    }
    if cd_b == 0 && doubled_point_on_segment(b, c, d) {
        return true;
    }
    (ab_c > 0) != (ab_d > 0) && (cd_a > 0) != (cd_b > 0)
}

fn doubled_triangle_area2(a: (i128, i128), b: (i128, i128), c: (i128, i128)) -> i128 {
    let ab_x = b.0 - a.0;
    let ab_z = b.1 - a.1;
    let ac_x = c.0 - a.0;
    let ac_z = c.1 - a.1;
    ab_x * ac_z - ab_z * ac_x
}

fn doubled_point_on_segment(point: (i128, i128), start: (i128, i128), end: (i128, i128)) -> bool {
    point.0 >= start.0.min(end.0)
        && point.0 <= start.0.max(end.0)
        && point.1 >= start.1.min(end.1)
        && point.1 <= start.1.max(end.1)
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

    fn terminal_profile(x: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, 4.0, -4.0),
            Vector3::new(x, 4.1, -3.0),
            Vector3::new(x, 4.2, -1.0),
            Vector3::new(x, 4.0, 0.0),
            Vector3::new(x, 4.2, 1.0),
            Vector3::new(x, 4.1, 3.0),
            Vector3::new(x, 4.0, 4.0),
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
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[4],
                boundary_points_world[5],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[5],
                boundary_points_world[6],
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
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
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

    fn terminal_input_with_endpoint_x(endpoint_x: f32) -> NodeArrangementInput {
        let mouth = OrderedIncidentPieceMouth {
            profile: terminal_profile(10.0),
            endpoint_profile: terminal_profile(endpoint_x),
            boundary_paths_world: Vec::new(),
            band_start_paths_world: Vec::new(),
            band_end_paths_world: Vec::new(),
            uses_sampled_band_domain_paths: false,
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::Terminal,
            &[mouth],
        )
        .expect("test terminal mouth should produce canonical input")
    }

    fn nonterminal_input_with_side_join_candidate() -> NodeArrangementInput {
        let mut input = input_with_endpoint_x(0.0);
        input.mouths[0]
            .terminal_end_bands
            .push(NodeInputTerminalEndBand {
                source_band_index: 3,
                band_kind: RoadSurfaceBandKind::Sidewalk,
                boundary_mode: NodeInputTerminalEndBandBoundaryMode::MaterialBand,
                inner_start_world: RoadVec3::new(0.0, 4.4, 4.0),
                inner_end_world: RoadVec3::new(2.0, 4.4, 4.0),
                outer_start_world: RoadVec3::new(0.0, 4.4, 6.0),
                outer_end_world: RoadVec3::new(2.0, 4.4, 6.0),
                contour_world: vec![
                    RoadVec3::new(0.0, 4.4, 4.0),
                    RoadVec3::new(2.0, 4.4, 4.0),
                    RoadVec3::new(2.0, 4.4, 6.0),
                    RoadVec3::new(0.0, 4.4, 6.0),
                ],
            });
        input
    }

    fn nonterminal_input_with_same_owner_side_join_cap() -> NodeArrangementInput {
        let mut input = input_with_endpoint_x(0.0);
        input.mouths[0]
            .terminal_end_bands
            .push(NodeInputTerminalEndBand {
                source_band_index: 3,
                band_kind: RoadSurfaceBandKind::Sidewalk,
                boundary_mode: NodeInputTerminalEndBandBoundaryMode::SameOwnerOuterCap,
                inner_start_world: RoadVec3::new(0.0, 4.4, 4.0),
                inner_end_world: RoadVec3::new(2.0, 4.4, 4.0),
                outer_start_world: RoadVec3::new(0.9, 4.4, 6.0),
                outer_end_world: RoadVec3::new(1.1, 4.4, 6.0),
                contour_world: vec![
                    RoadVec3::new(0.0, 4.4, 4.0),
                    RoadVec3::new(2.0, 4.4, 4.0),
                    RoadVec3::new(1.0, 4.4, 6.0),
                ],
            });
        input
    }

    fn bend_input_with_curb_within_footprint(
        boundary_mode: NodeInputTerminalEndBandBoundaryMode,
    ) -> NodeArrangementInput {
        let mut input = input_with_endpoint_x(0.0);
        input.piece_kind = RoadSurfaceVisualNodePieceKind::Bend;
        input.mouths[0]
            .terminal_end_bands
            .push(NodeInputTerminalEndBand {
                source_band_index: 1,
                band_kind: RoadSurfaceBandKind::CurbOrShoulder,
                boundary_mode,
                inner_start_world: RoadVec3::new(0.0, 4.2, -2.0),
                inner_end_world: RoadVec3::new(2.0, 4.2, -2.0),
                outer_start_world: RoadVec3::new(0.0, 4.2, -5.0),
                outer_end_world: RoadVec3::new(2.0, 4.2, -5.0),
                contour_world: vec![
                    RoadVec3::new(0.0, 4.2, -5.0),
                    RoadVec3::new(2.0, 4.2, -5.0),
                    RoadVec3::new(2.0, 4.2, -2.0),
                    RoadVec3::new(0.0, 4.2, -2.0),
                ],
            });
        input
    }

    fn bend_input_with_curb_guard_within_footprint() -> NodeArrangementInput {
        bend_input_with_curb_within_footprint(
            NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint,
        )
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
    fn nonterminal_side_join_end_bands_emit_canonical_ownership_candidates() {
        let contours =
            NodeRailContourSet::from_input(&nonterminal_input_with_side_join_candidate())
                .expect("valid contours");

        assert!(contours.contours.iter().any(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
                && contour.source_mouth_order_index == 0
        }));
        assert!(contours.contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Sidewalk,
                }
                && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && contour.source_mouth_order_index == 0
                && contour.source_band_index == Some(3)
        }));
        assert!(contours.constraints.iter().any(|constraint| {
            matches!(
                constraint.kind,
                NodeRailConstraintKind::FootprintSeam {
                    adjacent_kind: RoadSurfaceBandKind::Sidewalk
                }
            ) && constraint.source_band_index == Some(3)
        }));
    }

    #[test]
    fn bend_curb_guard_end_bands_contribute_canonical_footprint() {
        let contours =
            NodeRailContourSet::from_input(&bend_input_with_curb_guard_within_footprint())
                .expect("valid contours");
        let guard_outer_key = road_point_key(RoadVec2::new(0.0, -5.0));

        assert!(contours.contours.iter().any(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
                && contour
                    .points_xz
                    .iter()
                    .any(|point| road_point_key(*point) == guard_outer_key)
        }));
        assert!(contours.contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                }
                && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && contour.source_band_index == Some(1)
                && contour
                    .points_xz
                    .iter()
                    .any(|point| road_point_key(*point) == guard_outer_key)
        }));
    }

    #[test]
    fn bend_within_footprint_curb_bands_do_not_emit_fallback_asphalt_contacts() {
        for boundary_mode in [
            NodeInputTerminalEndBandBoundaryMode::MaterialBandWithinFootprint,
            NodeInputTerminalEndBandBoundaryMode::CurbGuardWithinFootprint,
        ] {
            let contours = NodeRailContourSet::from_input(&bend_input_with_curb_within_footprint(
                boundary_mode,
            ))
            .expect("valid contours");

            assert!(!contours.constraints.iter().any(|constraint| {
                constraint.owner.zip(constraint.opposite_owner).is_some_and(
                    |(owner, opposite_owner)| {
                        constraint.kind == NodeRailConstraintKind::AsphaltCurbContact
                            && constraint.source_band_index == Some(1)
                            && ((owner.kind() == RoadSurfaceBandKind::CurbOrShoulder
                                && opposite_owner.kind() == RoadSurfaceBandKind::Carriageway)
                                || (owner.kind() == RoadSurfaceBandKind::Carriageway
                                    && opposite_owner.kind()
                                        == RoadSurfaceBandKind::CurbOrShoulder))
                    },
                )
            }));
            assert!(contours.constraints.iter().any(|constraint| {
                constraint.kind == NodeRailConstraintKind::AsphaltCurbContact
                    && constraint.source_band_index == Some(1)
                    && constraint
                        .owner
                        .is_some_and(|owner| owner.kind() == RoadSurfaceBandKind::CurbOrShoulder)
                    && constraint.opposite_owner.is_none()
            }));
        }
    }

    #[test]
    fn generated_asphalt_curb_contact_can_take_authority_from_carriageway_boundary() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(2.0, 0.0),
                RoadVec2::new(2.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(actual_curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(0.0, 1.0),
                RoadVec2::new(2.0, 1.0),
                RoadVec2::new(2.0, 2.0),
                RoadVec2::new(0.0, 2.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::AsphaltCurbContact,
            source_mouth_order_index: 0,
            source_band_index: None,
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(source_curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(2.0, 1.0)],
        });

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        assert!(constraints.iter().any(|constraint| {
            let start = road_point_key(RoadVec2::new(0.0, 1.0));
            let end = road_point_key(RoadVec2::new(2.0, 1.0));
            constraint.kind == NodeRailConstraintKind::AsphaltCurbContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn generated_asphalt_curb_contact_splits_carriageway_boundary_at_overlay_contact() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            0,
            Some(1),
            Some(actual_curb_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            vec![
                RoadVec2::new(3.0, 0.5),
                RoadVec2::new(4.0, 0.5),
                RoadVec2::new(4.0, 1.5),
                RoadVec2::new(3.0, 1.5),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
        constraints.push(NodeRailConstraint {
            constraint_index: constraints.len(),
            kind: NodeRailConstraintKind::AsphaltCurbContact,
            source_mouth_order_index: 0,
            source_band_index: None,
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(source_curb_owner),
            points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
        });

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        let start = road_point_key(RoadVec2::new(3.0, 1.0));
        let end = road_point_key(RoadVec2::new(4.0, 1.0));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::AsphaltCurbContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn generated_asphalt_curb_contact_uses_owner_group_union_for_split_domains() {
        let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
        let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
        let mut contours = Vec::new();
        let mut constraints = Vec::new();

        push_generated_contour(
            NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            0,
            Some(0),
            Some(asphalt_owner),
            NodeGeneratedContourClaimPriority::MouthBand,
            NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::Carriageway,
            },
            vec![
                RoadVec2::new(0.0, 0.0),
                RoadVec2::new(4.0, 0.0),
                RoadVec2::new(4.0, 1.0),
                RoadVec2::new(0.0, 1.0),
            ],
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("asphalt contour is valid");
        for points in [
            vec![
                RoadVec2::new(2.0, 0.5),
                RoadVec2::new(3.2, 0.5),
                RoadVec2::new(3.2, 1.5),
                RoadVec2::new(2.0, 1.5),
            ],
            vec![
                RoadVec2::new(2.8, 0.5),
                RoadVec2::new(4.0, 0.5),
                RoadVec2::new(4.0, 1.5),
                RoadVec2::new(2.8, 1.5),
            ],
        ] {
            push_generated_contour(
                NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                },
                0,
                Some(1),
                Some(actual_curb_owner),
                NodeGeneratedContourClaimPriority::MouthBand,
                NodeRailConstraintKind::BandContour {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                },
                points,
                None,
                &mut contours,
                &mut constraints,
            )
            .expect("curb contour is valid");
        }

        append_generated_same_band_contact_constraints(
            RoadSurfaceVisualNodePieceKind::Bend,
            &contours,
            &mut constraints,
        );

        let start = road_point_key(RoadVec2::new(2.0, 1.0));
        let end = road_point_key(RoadVec2::new(4.0, 1.0));
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::AsphaltCurbContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == start
                && road_point_key(constraint.points_xz[1]) == end
        }));
    }

    #[test]
    fn nonterminal_same_owner_caps_emit_canonical_side_join_fill() {
        let contours =
            NodeRailContourSet::from_input(&nonterminal_input_with_same_owner_side_join_cap())
                .expect("valid contours");
        let cap_tip = road_point_key(RoadVec2::new(1.0, 6.0));

        assert!(
            NodeGeneratedContourClaimPriority::SideJoin
                < NodeGeneratedContourClaimPriority::MouthBand,
            "side-join candidates must remain whole so their protected footprint seam survives ownership cleanup"
        );
        assert!(contours.contours.iter().any(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                && contour.claim_priority == NodeGeneratedContourClaimPriority::Footprint
                && contour
                    .points_xz
                    .iter()
                    .any(|point| road_point_key(*point) == cap_tip)
        }));
        assert!(contours.contours.iter().any(|contour| {
            contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Sidewalk,
                }
                && contour.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
                && contour.source_band_index == Some(3)
                && contour
                    .points_xz
                    .iter()
                    .any(|point| road_point_key(*point) == cap_tip)
        }));
    }

    #[test]
    fn terminal_curb_end_band_asphalt_contacts_name_adjacent_carriageways() {
        let input = terminal_input_with_endpoint_x(0.0);
        let terminal_curb_source = input.mouths[0].band_intervals.len();
        let contours = NodeRailContourSet::from_input(&input).expect("valid terminal contours");
        let left_carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
        let right_carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 3);
        let left_segment = GeneratedContourEdgeKey::new(
            road_point_key(RoadVec2::new(0.0, -1.0)),
            road_point_key(RoadVec2::new(0.0, 0.0)),
        );
        let right_segment = GeneratedContourEdgeKey::new(
            road_point_key(RoadVec2::new(0.0, 0.0)),
            road_point_key(RoadVec2::new(0.0, 1.0)),
        );
        let contacts = contours
            .constraints
            .iter()
            .filter(|constraint| {
                constraint.kind == NodeRailConstraintKind::AsphaltCurbContact
                    && constraint.source_band_index == Some(terminal_curb_source)
                    && constraint
                        .owner
                        .is_some_and(|owner| owner.kind() == RoadSurfaceBandKind::CurbOrShoulder)
            })
            .filter_map(|constraint| {
                let opposite_owner = constraint.opposite_owner?;
                Some((
                    GeneratedContourEdgeKey::new(
                        road_point_key(constraint.points_xz[0]),
                        road_point_key(constraint.points_xz[1]),
                    ),
                    opposite_owner,
                ))
            })
            .collect::<BTreeSet<_>>();

        assert!(contacts.contains(&(left_segment, left_carriageway)));
        assert!(contacts.contains(&(right_segment, right_carriageway)));
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
