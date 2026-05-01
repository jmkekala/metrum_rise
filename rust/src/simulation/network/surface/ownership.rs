//! Boolean ownership solve for canonical node-arrangement contours.

#![allow(dead_code)]

use super::arrangement::{NodeBandOwner, NodeHeightSource};
use super::backend::road_vec2_to_overlay_point;
use super::rails::{NodeGeneratedContour, NodeGeneratedContourKind, NodeRailContourSet};
use super::{
    NodeOverlayContour, NodeOverlayShape, NodeOverlayShapes, RoadSurfaceBandKind,
    RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use i_overlay::core::overlay_rule::OverlayRule;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnership {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) footprint_shapes: NodeOverlayShapes,
    pub(crate) asphalt_shapes: NodeOverlayShapes,
    pub(crate) non_road_shapes: NodeOverlayShapes,
    pub(crate) owned_regions: Vec<NodeBooleanOwnedRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeBooleanOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner: NodeBandOwner,
    pub(crate) source_mouth_order_index: usize,
    pub(crate) source_band_index: Option<usize>,
    pub(crate) shape: NodeOverlayShape,
    pub(crate) area_m2: f32,
    pub(crate) height_sources: Vec<NodeHeightSource>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeBooleanOwnershipError {
    EmptyContourSet {
        node_id: u32,
    },
    EmptyFootprint {
        node_id: u32,
    },
    MissingBandOwner {
        mouth_order_index: usize,
        band_index: Option<usize>,
    },
    BooleanOperationFailed {
        stage: &'static str,
    },
    UnownedAsphaltResidual {
        shape_count: usize,
        area_m2: f32,
    },
    UnownedBandResidual {
        kind: RoadSurfaceBandKind,
        shape_count: usize,
        area_m2: f32,
    },
    UnownedNonRoadResidual {
        shape_count: usize,
        area_m2: f32,
    },
}

struct OwnedDomainResult {
    regions: Vec<NodeBooleanOwnedRegion>,
    claimed_shapes: NodeOverlayShapes,
}

impl RoadSurfaceSystem {
    pub(super) fn build_node_boolean_ownership_from_rails(
        rails: &NodeRailContourSet,
    ) -> Result<NodeBooleanOwnership, NodeBooleanOwnershipError> {
        NodeBooleanOwnership::from_rails(rails)
    }
}

impl NodeBooleanOwnership {
    pub(crate) fn from_rails(
        rails: &NodeRailContourSet,
    ) -> Result<Self, NodeBooleanOwnershipError> {
        if rails.contours.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyContourSet {
                node_id: rails.node_id,
            });
        }

        let footprint_contours = overlay_contours_for_domains(rails, |contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
        });
        let mut footprint_shapes = overlay_union(&footprint_contours, "footprint_union")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut footprint_shapes);
        if footprint_shapes.is_empty() {
            return Err(NodeBooleanOwnershipError::EmptyFootprint {
                node_id: rails.node_id,
            });
        }

        let asphalt_domains = domains_for_band_kind(rails, RoadSurfaceBandKind::Carriageway);
        let asphalt_contours = asphalt_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut asphalt_shapes = overlay_union(&asphalt_contours, "asphalt_union")?;
        asphalt_shapes = overlay_intersect(
            &asphalt_shapes,
            &footprint_shapes,
            "asphalt_clip_to_footprint",
        )?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut asphalt_shapes);

        let mut non_road_shapes =
            overlay_difference(&footprint_shapes, &asphalt_shapes, "non_road_difference")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut non_road_shapes);

        let mut owned_regions = Vec::new();
        let asphalt_result =
            owned_regions_from_domains(&asphalt_shapes, &asphalt_domains, ResidualKind::Asphalt)?;
        owned_regions.extend(asphalt_result.regions);

        let non_road_result = split_non_road_regions(&non_road_shapes, rails)?;
        owned_regions.extend(non_road_result.regions);
        let non_road_residual = overlay_difference(
            &non_road_shapes,
            &non_road_result.claimed_shapes,
            "non_road_residual",
        )?;
        reject_residual(non_road_residual, ResidualKind::NonRoad)?;

        sort_boolean_owned_regions(&mut owned_regions);
        Ok(Self {
            node_id: rails.node_id,
            piece_kind: rails.piece_kind,
            footprint_shapes,
            asphalt_shapes,
            non_road_shapes,
            owned_regions,
        })
    }
}

fn split_non_road_regions(
    non_road_shapes: &NodeOverlayShapes,
    rails: &NodeRailContourSet,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();

    for kind in non_road_band_order() {
        let kind_domains = domains_for_band_kind(rails, kind);
        if kind_domains.is_empty() {
            continue;
        }

        let kind_contours = kind_domains
            .iter()
            .map(|domain| overlay_contour_from_domain(domain))
            .collect::<Vec<_>>();
        let mut kind_target = overlay_union(&kind_contours, "non_road_band_union")?;
        kind_target = overlay_intersect(
            &kind_target,
            non_road_shapes,
            "non_road_band_clip_to_target",
        )?;
        kind_target = overlay_difference(&kind_target, &claimed_shapes, "non_road_band_unclaimed")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut kind_target);
        if kind_target.is_empty() {
            continue;
        }

        let kind_result =
            owned_regions_from_domains(&kind_target, &kind_domains, ResidualKind::Band(kind))?;
        claimed_shapes =
            overlay_union_shape_sets(&claimed_shapes, &kind_result.claimed_shapes, "claim_union")?;
        regions.extend(kind_result.regions);
    }

    RoadSurfaceSystem::sort_overlay_shapes(&mut claimed_shapes);
    Ok(OwnedDomainResult {
        regions,
        claimed_shapes,
    })
}

fn owned_regions_from_domains(
    target_shapes: &NodeOverlayShapes,
    domains: &[&NodeGeneratedContour],
    residual_kind: ResidualKind,
) -> Result<OwnedDomainResult, NodeBooleanOwnershipError> {
    if target_shapes.is_empty() {
        return Ok(OwnedDomainResult {
            regions: Vec::new(),
            claimed_shapes: Vec::new(),
        });
    }

    let mut regions = Vec::new();
    let mut claimed_shapes = Vec::new();

    for domain in domains {
        let owner = domain
            .owner
            .ok_or(NodeBooleanOwnershipError::MissingBandOwner {
                mouth_order_index: domain.source_mouth_order_index,
                band_index: domain.source_band_index,
            })?;
        let domain_contour = overlay_contour_from_domain(domain);
        let mut domain_shapes = overlay_union(&[domain_contour], "domain_union")?;
        domain_shapes = overlay_intersect(&domain_shapes, target_shapes, "domain_clip")?;
        domain_shapes = overlay_difference(&domain_shapes, &claimed_shapes, "domain_unclaimed")?;
        RoadSurfaceSystem::sort_overlay_shapes(&mut domain_shapes);
        if domain_shapes.is_empty() {
            continue;
        }

        for shape in &domain_shapes {
            regions.push(NodeBooleanOwnedRegion {
                kind: band_kind(domain).expect("owned domain must be a band contour"),
                owner,
                source_mouth_order_index: domain.source_mouth_order_index,
                source_band_index: domain.source_band_index,
                shape: shape.clone(),
                area_m2: RoadSurfaceSystem::overlay_shape_area_m2(shape),
                height_sources: canonical_height_sources(domain.height_sources.iter().cloned()),
            });
        }
        claimed_shapes =
            overlay_union_shape_sets(&claimed_shapes, &domain_shapes, "domain_claim_union")?;
    }

    let residual = overlay_difference(target_shapes, &claimed_shapes, "domain_residual")?;
    reject_residual(residual, residual_kind)?;
    Ok(OwnedDomainResult {
        regions,
        claimed_shapes,
    })
}

fn overlay_contours_for_domains(
    rails: &NodeRailContourSet,
    predicate: impl Fn(&NodeGeneratedContour) -> bool,
) -> Vec<NodeOverlayContour> {
    rails
        .contours
        .iter()
        .filter(|contour| predicate(contour))
        .map(overlay_contour_from_domain)
        .collect()
}

fn domains_for_band_kind(
    rails: &NodeRailContourSet,
    kind: RoadSurfaceBandKind,
) -> Vec<&NodeGeneratedContour> {
    rails
        .contours
        .iter()
        .filter(|contour| band_kind(contour) == Some(kind))
        .collect()
}

fn overlay_contour_from_domain(domain: &NodeGeneratedContour) -> NodeOverlayContour {
    domain
        .points_xz
        .iter()
        .copied()
        .map(road_vec2_to_overlay_point)
        .collect()
}

fn overlay_union(
    contours: &[NodeOverlayContour],
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut shapes = RoadSurfaceSystem::overlay_union_contours(contours)
        .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_intersect(
    subject: &NodeOverlayShapes,
    clip: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    if subject.is_empty() || clip.is_empty() {
        return Ok(Vec::new());
    }
    let mut shapes =
        RoadSurfaceSystem::overlay_binary_shapes(subject, clip, OverlayRule::Intersect)
            .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_difference(
    subject: &NodeOverlayShapes,
    clip: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    let mut shapes =
        RoadSurfaceSystem::overlay_binary_shapes(subject, clip, OverlayRule::Difference)
            .ok_or(NodeBooleanOwnershipError::BooleanOperationFailed { stage })?;
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    Ok(shapes)
}

fn overlay_union_shape_sets(
    existing: &NodeOverlayShapes,
    added: &NodeOverlayShapes,
    stage: &'static str,
) -> Result<NodeOverlayShapes, NodeBooleanOwnershipError> {
    if existing.is_empty() {
        return Ok(added.clone());
    }
    if added.is_empty() {
        return Ok(existing.clone());
    }
    let contours = existing
        .iter()
        .chain(added.iter())
        .flat_map(|shape| shape.iter().cloned())
        .collect::<Vec<_>>();
    overlay_union(&contours, stage)
}

fn reject_residual(
    residual: NodeOverlayShapes,
    residual_kind: ResidualKind,
) -> Result<(), NodeBooleanOwnershipError> {
    if residual.is_empty() {
        return Ok(());
    }

    let shape_count = residual.len();
    let area_m2 = residual
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum();
    match residual_kind {
        ResidualKind::Asphalt => Err(NodeBooleanOwnershipError::UnownedAsphaltResidual {
            shape_count,
            area_m2,
        }),
        ResidualKind::Band(kind) => Err(NodeBooleanOwnershipError::UnownedBandResidual {
            kind,
            shape_count,
            area_m2,
        }),
        ResidualKind::NonRoad => Err(NodeBooleanOwnershipError::UnownedNonRoadResidual {
            shape_count,
            area_m2,
        }),
    }
}

fn band_kind(contour: &NodeGeneratedContour) -> Option<RoadSurfaceBandKind> {
    match contour.kind {
        NodeGeneratedContourKind::Band { kind } => Some(kind),
        NodeGeneratedContourKind::FullRoadbed => None,
    }
}

fn non_road_band_order() -> [RoadSurfaceBandKind; 7] {
    [
        RoadSurfaceBandKind::CurbOrShoulder,
        RoadSurfaceBandKind::Sidewalk,
        RoadSurfaceBandKind::Footpath,
        RoadSurfaceBandKind::CycleTrack,
        RoadSurfaceBandKind::Median,
        RoadSurfaceBandKind::Parking,
        RoadSurfaceBandKind::TramReservation,
    ]
}

fn sort_boolean_owned_regions(regions: &mut [NodeBooleanOwnedRegion]) {
    regions.sort_by(|a, b| {
        RoadSurfaceSystem::band_kind_sort_key(a.kind)
            .cmp(&RoadSurfaceSystem::band_kind_sort_key(b.kind))
            .then(a.source_mouth_order_index.cmp(&b.source_mouth_order_index))
            .then(a.source_band_index.cmp(&b.source_band_index))
            .then(a.area_m2.total_cmp(&b.area_m2))
    });
}

fn canonical_height_sources(
    sources: impl IntoIterator<Item = NodeHeightSource>,
) -> Vec<NodeHeightSource> {
    let mut sources = sources.into_iter().collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources
}

#[derive(Clone, Copy)]
enum ResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::input::NodeArrangementInput;
    use crate::simulation::network::surface::rails::NodeRailContourSet;
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

    fn contour_set() -> NodeRailContourSet {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0),
            endpoint_profile: profile(0.0),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input");
        NodeRailContourSet::from_input(&input).expect("test input should produce contours")
    }

    #[test]
    fn boolean_ownership_produces_asphalt_and_band_owned_regions() {
        let ownership =
            NodeBooleanOwnership::from_rails(&contour_set()).expect("valid ownership solve");

        assert_eq!(ownership.node_id, 42);
        assert_eq!(
            ownership.piece_kind,
            RoadSurfaceVisualNodePieceKind::JunctionN
        );
        assert_eq!(ownership.footprint_shapes.len(), 1);
        assert_eq!(ownership.asphalt_shapes.len(), 1);
        assert_eq!(ownership.non_road_shapes.len(), 2);
        assert_eq!(ownership.owned_regions.len(), 4);
        assert!(
            ownership
                .owned_regions
                .iter()
                .any(|region| region.kind == RoadSurfaceBandKind::Carriageway
                    && region.owner == NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2)
                    && !region.height_sources.is_empty())
        );
        assert_eq!(
            ownership
                .owned_regions
                .iter()
                .filter(|region| region.kind == RoadSurfaceBandKind::Sidewalk)
                .count(),
            2
        );
    }

    #[test]
    fn boolean_ownership_rejects_unowned_non_road_residual() {
        let mut rails = contour_set();
        rails.contours.retain(|contour| {
            contour.kind == NodeGeneratedContourKind::FullRoadbed
                || contour.kind
                    == NodeGeneratedContourKind::Band {
                        kind: RoadSurfaceBandKind::Carriageway,
                    }
        });

        let error = NodeBooleanOwnership::from_rails(&rails)
            .expect_err("non-road footprint without band contours must be rejected");

        assert!(matches!(
            error,
            NodeBooleanOwnershipError::UnownedNonRoadResidual { .. }
        ));
    }
}
