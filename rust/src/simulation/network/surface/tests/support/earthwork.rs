// SPDX-License-Identifier: GPL-2.0-only

//! Earthwork provenance and footprint helpers for road-surface tests.

use super::*;

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::tests) struct FootprintOverflowMetrics {
    pub(in crate::simulation::network::surface::tests) max_overflow_m: f32,
}

pub(in crate::simulation::network::surface::tests) fn footprint_sample_offsets(
    section: &RoadSurfaceSection,
) -> Vec<f32> {
    let mut offsets = Vec::new();
    for band in &section.bands {
        if !matches!(
            band.kind,
            super::RoadSurfaceBandKind::Carriageway
                | super::RoadSurfaceBandKind::CurbOrShoulder
                | super::RoadSurfaceBandKind::Sidewalk
                | super::RoadSurfaceBandKind::Footpath
        ) {
            continue;
        }
        offsets.push(band.lateral_start_m);
        offsets.push((band.lateral_start_m + band.lateral_end_m) * 0.5);
        offsets.push(band.lateral_end_m);
    }
    offsets.sort_by(|a, b| a.total_cmp(b));
    offsets.dedup_by(|a, b| (*a - *b).abs() <= 0.001);
    offsets
}

pub(in crate::simulation::network::surface::tests) fn measure_max_footprint_overflow(
    surface: &RoadSurfaceSystem,
    graph: &RegionGraph,
    edge_idx: usize,
    terrain: &TerrainSystem,
) -> FootprintOverflowMetrics {
    let mut best = FootprintOverflowMetrics {
        max_overflow_m: f32::NEG_INFINITY,
    };

    let sections = surface.compiled_sections().get(&edge_idx).unwrap();
    for section in sections.iter() {
        for lateral_offset_m in footprint_sample_offsets(section) {
            let Some(road_height_m) = section_height_at_lateral_offset(section, lateral_offset_m)
            else {
                continue;
            };
            let sample_x = section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset_m);
            let sample_z = section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset_m);
            let visual_height_m = surface
                .sample_paved_support_height(graph, terrain, sample_x as f32, sample_z as f32)
                .unwrap_or_else(|| {
                    panic!(
                        "paved footprint sample must have road-owned support: edge_idx={edge_idx} s_m={:.3} lateral={lateral_offset_m:.3} sample=({sample_x:.3},{sample_z:.3})",
                        section.s_m
                    )
                });
            let overflow_m = visual_height_m - road_height_m;
            if overflow_m > best.max_overflow_m {
                best = FootprintOverflowMetrics {
                    max_overflow_m: overflow_m,
                };
            }
        }
    }

    best
}

pub(in crate::simulation::network::surface::tests) fn assert_earthwork_faces_stay_outside_top_footprint(
    piece: &RoadSurfaceVisualNodePiece,
) {
    let top_contours = overlay_contours_from_top_polygons(
        piece
            .road_surface_polygons
            .iter()
            .chain(piece.curb_surface_polygons.iter())
            .chain(piece.sidewalk_surface_polygons.iter()),
    );
    let top_shapes = RoadSurfaceSystem::overlay_union_contours(&top_contours)
        .expect("node top overlay union should succeed");
    for face in &piece.render_earthwork_faces {
        let face_contour = overlay_contour_from_world_points(&face.polygon.points_world);
        if face_contour.len() < 3 {
            continue;
        }
        let face_shapes = RoadSurfaceSystem::overlay_union_contours(&[face_contour])
            .expect("earthwork face overlay union should succeed");
        let overlap = RoadSurfaceSystem::overlay_binary_shapes(
            &face_shapes,
            &top_shapes,
            OverlayRule::Intersect,
        )
        .expect("earthwork/top overlap check should succeed");
        let overlap_area_m2 = overlay_area_m2(&overlap);
        let budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&face_shapes)
            .max(RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(
                &top_shapes,
            ));
        assert!(
            overlap_area_m2 <= budget_m2,
            "earthwork face must not intrude into road-owned top footprint; kind={:?} inner={:?}->{:?} face={:?} overlap_area={overlap_area_m2:.6} budget={budget_m2:.6}",
            piece.kind,
            face.inner_start,
            face.inner_end,
            face.polygon.points_world
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_earthwork_faces_have_footprint_provenance(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "node earthwork faces should be generated from owned footprint boundaries"
    );
    for face in &piece.render_earthwork_faces {
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind,
            owner_index,
            boundary_source,
        } = face.source
        else {
            panic!(
                "node earthwork face must carry node footprint provenance, got {:?}",
                face.source
            );
        };
        assert_eq!(node_id, piece.node_id);
        assert_eq!(kind, piece.kind);
        assert!(
            piece
                .owned_regions
                .iter()
                .any(|region| region.kind == owner_kind && region.owner_index == owner_index),
            "node earthwork face owner must refer to a canonical owned top region"
        );
        let boundary_source = boundary_source
            .expect("node earthwork face must carry exact boundary endpoint provenance");
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.start);
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.end);
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_footprint_boundary_vertex_source_is_valid(
    piece: &RoadSurfaceVisualNodePiece,
    source: NodeFootprintBoundaryVertexSource,
) {
    match source {
        NodeFootprintBoundaryVertexSource::Direct(direct) => {
            assert!(
                direct.top_surface_source_index < piece.node_top_surface_sources.len(),
                "direct boundary source must reference an emitted top surface source"
            );
            assert!(
                direct.grade_authority_index < piece.node_grade_authorities.len(),
                "direct boundary source must reference node grade authority"
            );
        }
        NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint { .. } => {}
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start,
            owning_segment_end,
            ..
        } => {
            assert_node_footprint_boundary_vertex_source_is_valid(
                piece,
                NodeFootprintBoundaryVertexSource::Direct(owning_segment_start),
            );
            assert_node_footprint_boundary_vertex_source_is_valid(
                piece,
                NodeFootprintBoundaryVertexSource::Direct(owning_segment_end),
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_span_earthwork_faces_have_support_provenance(
    piece: &super::RoadSurfaceVisualSpanPiece,
    edge_idx: usize,
    edge_class: EdgeClass,
) {
    assert!(
        !piece.render_earthwork_faces.is_empty(),
        "span earthwork faces should be generated from span support region boundaries"
    );
    let expected_policy = RoadSurfaceEarthworkSupportPolicy::from_edge_class(edge_class);
    for face in &piece.render_earthwork_faces {
        let RoadSurfaceEarthworkFaceSource::SpanSupportBoundary {
            edge_idx: source_edge_idx,
            edge_class: source_edge_class,
            support_policy,
            owner,
            role,
            start_section_index,
            end_section_index,
            start_s_m,
            end_s_m,
        } = face.source
        else {
            panic!(
                "span earthwork face must carry span support provenance, got {:?}",
                face.source
            );
        };
        assert_eq!(source_edge_idx, edge_idx);
        assert_eq!(source_edge_class, edge_class);
        assert_eq!(support_policy, expected_policy);
        assert!(
            piece.span_earthwork_support_regions.iter().any(|region| {
                region.edge_idx == source_edge_idx
                    && region.owner == owner
                    && region.role == role
                    && region.start_section_index == start_section_index
                    && region.end_section_index == end_section_index
                    && (region.start_s_m - start_s_m).abs() <= SAMPLE_EPSILON_M
                    && (region.end_s_m - end_s_m).abs() <= SAMPLE_EPSILON_M
            }),
            "span earthwork face source must refer to a stored support region"
        );
    }
}

pub(in crate::simulation::network::surface::tests) fn node_earthwork_face_edge_class(
    piece: &RoadSurfaceVisualNodePiece,
    source: RoadSurfaceEarthworkFaceSource,
) -> Option<EdgeClass> {
    let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
        owner_kind,
        owner_index,
        ..
    } = source
    else {
        return None;
    };
    piece
        .earthwork_owner_sources
        .iter()
        .find(|owner_source| {
            owner_source.owner_kind == owner_kind && owner_source.owner_index == owner_index
        })
        .map(|owner_source| owner_source.edge_class)
}
