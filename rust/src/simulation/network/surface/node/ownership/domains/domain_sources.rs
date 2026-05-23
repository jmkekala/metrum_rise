//! Domain contour selection helpers.

use super::*;

pub(in crate::simulation::network::surface::node::ownership) fn overlay_contours_for_domains(
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

pub(in crate::simulation::network::surface::node::ownership) fn asphalt_authority_domains(
    rails: &NodeRailContourSet,
) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, RoadSurfaceBandKind::Carriageway, |contour| {
        contour.contributes_to_asphalt()
    })
}

pub(in crate::simulation::network::surface::node::ownership) fn asphalt_owner_domains(
    rails: &NodeRailContourSet,
) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, RoadSurfaceBandKind::Carriageway, |contour| {
        contour.claims_asphalt_owner_region()
    })
}

pub(super) fn non_road_domains_for_band_kind(
    rails: &NodeRailContourSet,
    kind: RoadSurfaceBandKind,
) -> Vec<&NodeGeneratedContour> {
    domains_for_band_kind_matching(rails, kind, |contour| {
        contour.contributes_to_non_road_band()
    })
}

fn domains_for_band_kind_matching(
    rails: &NodeRailContourSet,
    kind: RoadSurfaceBandKind,
    predicate: impl Fn(&NodeGeneratedContour) -> bool,
) -> Vec<&NodeGeneratedContour> {
    let mut domains = rails
        .contours
        .iter()
        .filter(|contour| band_kind(contour) == Some(kind) && predicate(contour))
        .collect::<Vec<_>>();
    domains.sort_by_key(|contour| {
        (
            contour.claim_priority,
            contour.purpose,
            contour.source_mouth_order_index,
            contour.source_band_index,
        )
    });
    domains
}

pub(super) fn band_kind(contour: &NodeGeneratedContour) -> Option<RoadSurfaceBandKind> {
    match contour.kind {
        NodeGeneratedContourKind::Band { kind } => Some(kind),
        NodeGeneratedContourKind::FullRoadbed => None,
    }
}

pub(super) fn non_road_band_order() -> [RoadSurfaceBandKind; 7] {
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
