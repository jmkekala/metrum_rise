//! Canonical ordering and key normalization for CDT provenance and edges.

use super::super::*;

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_road_constraint_source_cmp(
    a: TerrainCdtRoadConstraintSource,
    b: TerrainCdtRoadConstraintSource,
) -> std::cmp::Ordering {
    a.stable_piece_id
        .cmp(&b.stable_piece_id)
        .then(a.local_loop_index.cmp(&b.local_loop_index))
        .then(a.local_edge_index.cmp(&b.local_edge_index))
        .then_with(|| terrain_cdt_boundary_source_cmp(a.boundary_source, b.boundary_source))
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_optional_boundary_source_cmp(
    a: Option<TerrainCdtRoadBoundarySource>,
    b: Option<TerrainCdtRoadBoundarySource>,
) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => terrain_cdt_boundary_source_cmp(a, b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_boundary_sources_cmp(
    a: &[TerrainCdtRoadBoundarySource],
    b: &[TerrainCdtRoadBoundarySource],
) -> std::cmp::Ordering {
    for (source_a, source_b) in a.iter().copied().zip(b.iter().copied()) {
        let ordering = terrain_cdt_boundary_source_cmp(source_a, source_b);
        if !ordering.is_eq() {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_boundary_source_cmp(
    a: TerrainCdtRoadBoundarySource,
    b: TerrainCdtRoadBoundarySource,
) -> std::cmp::Ordering {
    let kind_order = a.source_kind_code().cmp(&b.source_kind_code());
    if !kind_order.is_eq() {
        return kind_order;
    }

    match (a, b) {
        (
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_a,
                edge_class: edge_class_a,
                support_policy: support_policy_a,
                source_band_index: source_band_index_a,
                band_kind: band_kind_a,
                role: role_a,
                start_section_index: start_section_index_a,
                end_section_index: end_section_index_a,
                start_s_m: start_s_m_a,
                end_s_m: end_s_m_a,
            },
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_idx: edge_idx_b,
                edge_class: edge_class_b,
                support_policy: support_policy_b,
                source_band_index: source_band_index_b,
                band_kind: band_kind_b,
                role: role_b,
                start_section_index: start_section_index_b,
                end_section_index: end_section_index_b,
                start_s_m: start_s_m_b,
                end_s_m: end_s_m_b,
            },
        ) => edge_idx_a
            .cmp(&edge_idx_b)
            .then(
                terrain_cdt_edge_class_sort_key(edge_class_a)
                    .cmp(&terrain_cdt_edge_class_sort_key(edge_class_b)),
            )
            .then(
                terrain_cdt_support_policy_sort_key(support_policy_a)
                    .cmp(&terrain_cdt_support_policy_sort_key(support_policy_b)),
            )
            .then(source_band_index_a.cmp(&source_band_index_b))
            .then(
                terrain_cdt_band_kind_sort_key(band_kind_a)
                    .cmp(&terrain_cdt_band_kind_sort_key(band_kind_b)),
            )
            .then(
                terrain_cdt_span_role_sort_key(role_a).cmp(&terrain_cdt_span_role_sort_key(role_b)),
            )
            .then(start_section_index_a.cmp(&start_section_index_b))
            .then(end_section_index_a.cmp(&end_section_index_b))
            .then(start_s_m_a.total_cmp(&start_s_m_b))
            .then(end_s_m_a.total_cmp(&end_s_m_b)),
        (
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_a,
                node_kind: node_kind_a,
                owner_kind: owner_kind_a,
                owner_index: owner_index_a,
                boundary_source: boundary_source_a,
            },
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
                node_id: node_id_b,
                node_kind: node_kind_b,
                owner_kind: owner_kind_b,
                owner_index: owner_index_b,
                boundary_source: boundary_source_b,
            },
        ) => node_id_a
            .cmp(&node_id_b)
            .then(
                terrain_cdt_node_kind_sort_key(node_kind_a)
                    .cmp(&terrain_cdt_node_kind_sort_key(node_kind_b)),
            )
            .then(
                terrain_cdt_band_kind_sort_key(owner_kind_a)
                    .cmp(&terrain_cdt_band_kind_sort_key(owner_kind_b)),
            )
            .then(owner_index_a.cmp(&owner_index_b))
            .then(boundary_source_a.cmp(&boundary_source_b)),
        (
            TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                node_id: node_id_a,
                node_kind: node_kind_a,
                owner_kind: owner_kind_a,
                owner_index_a: owner_index_a_a,
                owner_index_b: owner_index_b_a,
                boundary_source: boundary_source_a,
            },
            TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff {
                node_id: node_id_b,
                node_kind: node_kind_b,
                owner_kind: owner_kind_b,
                owner_index_a: owner_index_a_b,
                owner_index_b: owner_index_b_b,
                boundary_source: boundary_source_b,
            },
        ) => node_id_a
            .cmp(&node_id_b)
            .then(
                terrain_cdt_node_kind_sort_key(node_kind_a)
                    .cmp(&terrain_cdt_node_kind_sort_key(node_kind_b)),
            )
            .then(
                terrain_cdt_band_kind_sort_key(owner_kind_a)
                    .cmp(&terrain_cdt_band_kind_sort_key(owner_kind_b)),
            )
            .then(owner_index_a_a.cmp(&owner_index_a_b))
            .then(owner_index_b_a.cmp(&owner_index_b_b))
            .then(boundary_source_a.cmp(&boundary_source_b)),
        (
            TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a,
            },
            TerrainCdtRoadBoundarySource::BuildingSiteBoundary {
                building_idx: building_idx_b,
                local_loop_index: local_loop_index_b,
                local_edge_index: local_edge_index_b,
            },
        ) => building_idx_a
            .cmp(&building_idx_b)
            .then(local_loop_index_a.cmp(&local_loop_index_b))
            .then(local_edge_index_a.cmp(&local_edge_index_b)),
        (
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_a,
                local_loop_index: local_loop_index_a,
                local_edge_index: local_edge_index_a,
            },
            TerrainCdtRoadBoundarySource::SyntheticTestBoundary {
                stable_piece_id: stable_piece_id_b,
                local_loop_index: local_loop_index_b,
                local_edge_index: local_edge_index_b,
            },
        ) => stable_piece_id_a
            .cmp(&stable_piece_id_b)
            .then(local_loop_index_a.cmp(&local_loop_index_b))
            .then(local_edge_index_a.cmp(&local_edge_index_b)),
        _ => unreachable!("source kind codes must uniquely identify CDT source variants"),
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_edge_class_sort_key(
    edge_class: TerrainCdtEdgeClass,
) -> u8 {
    match edge_class {
        TerrainCdtEdgeClass::Standard => 0,
        TerrainCdtEdgeClass::Bridge => 1,
        TerrainCdtEdgeClass::Tunnel => 2,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_support_policy_sort_key(
    policy: TerrainCdtEarthworkSupportPolicy,
) -> u8 {
    match policy {
        TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan => 0,
        TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments => 1,
        TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals => 2,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_band_kind_sort_key(
    kind: TerrainCdtRoadBandKind,
) -> u8 {
    match kind {
        TerrainCdtRoadBandKind::Carriageway => 0,
        TerrainCdtRoadBandKind::CurbOrShoulder => 1,
        TerrainCdtRoadBandKind::Sidewalk => 2,
        TerrainCdtRoadBandKind::Footpath => 3,
        TerrainCdtRoadBandKind::Median => 4,
        TerrainCdtRoadBandKind::Parking => 5,
        TerrainCdtRoadBandKind::CycleTrack => 6,
        TerrainCdtRoadBandKind::TramReservation => 7,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_span_role_sort_key(
    role: TerrainCdtSpanRegionRole,
) -> u8 {
    match role {
        TerrainCdtSpanRegionRole::Asphalt => 0,
        TerrainCdtSpanRegionRole::CurbOrShoulder => 1,
        TerrainCdtSpanRegionRole::NonRoad => 2,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_node_kind_sort_key(
    kind: TerrainCdtNodePieceKind,
) -> u8 {
    match kind {
        TerrainCdtNodePieceKind::Terminal => 0,
        TerrainCdtNodePieceKind::Bend => 1,
        TerrainCdtNodePieceKind::JunctionN => 2,
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_edge_class_label(
    edge_class: TerrainCdtEdgeClass,
) -> &'static str {
    match edge_class {
        TerrainCdtEdgeClass::Standard => "standard",
        TerrainCdtEdgeClass::Bridge => "bridge",
        TerrainCdtEdgeClass::Tunnel => "tunnel",
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_support_policy_label(
    policy: TerrainCdtEarthworkSupportPolicy,
) -> &'static str {
    match policy {
        TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan => "standard_full_grounded_span",
        TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments => "bridge_endpoint_abutments",
        TerrainCdtEarthworkSupportPolicy::TunnelVisiblePortals => "tunnel_visible_portals",
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_band_kind_label(
    kind: TerrainCdtRoadBandKind,
) -> &'static str {
    match kind {
        TerrainCdtRoadBandKind::Carriageway => "carriageway",
        TerrainCdtRoadBandKind::CurbOrShoulder => "curb_or_shoulder",
        TerrainCdtRoadBandKind::Sidewalk => "sidewalk",
        TerrainCdtRoadBandKind::Footpath => "footpath",
        TerrainCdtRoadBandKind::Median => "median",
        TerrainCdtRoadBandKind::Parking => "parking",
        TerrainCdtRoadBandKind::CycleTrack => "cycle_track",
        TerrainCdtRoadBandKind::TramReservation => "tram_reservation",
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_span_role_label(
    role: TerrainCdtSpanRegionRole,
) -> &'static str {
    match role {
        TerrainCdtSpanRegionRole::Asphalt => "asphalt",
        TerrainCdtSpanRegionRole::CurbOrShoulder => "curb_or_shoulder",
        TerrainCdtSpanRegionRole::NonRoad => "non_road",
    }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_node_kind_label(
    kind: TerrainCdtNodePieceKind,
) -> &'static str {
    match kind {
        TerrainCdtNodePieceKind::Terminal => "terminal",
        TerrainCdtNodePieceKind::Bend => "bend",
        TerrainCdtNodePieceKind::JunctionN => "junction_n",
    }
}

pub(in crate::simulation::terrain::cdt) fn normalize_edge(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

pub(in crate::simulation::terrain::cdt) fn normalize_edge_array(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_vertex_xz_key(
    vertex: TerrainCdtVertex,
) -> (i64, i64) {
    (quantized_coord(vertex.x), quantized_coord(vertex.z))
}

pub(in crate::simulation::terrain::cdt) fn terrain_cdt_vertex_key(
    vertex: TerrainCdtVertex,
) -> (i64, i64, i64) {
    let (x_key, z_key) = terrain_cdt_vertex_xz_key(vertex);
    (x_key, z_key, quantized_coord(f64::from(vertex.height_m)))
}
