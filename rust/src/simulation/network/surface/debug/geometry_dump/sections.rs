//! Span section and ownership debug literal writers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface::debug) fn append_section_geometry_debug_dump(
        &self,
        dump: &mut String,
        terrain: &TerrainSystem,
        section: &RoadSurfaceSection,
    ) {
        let center_world = Vector3::new(
            section.center_xz.x,
            section.center_height_m,
            section.center_xz.y,
        );
        let source_center_y_m = terrain
            .sample_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;
        let visual_center_y_m = terrain
            .sample_visual_height_world(section.center_xz.x, section.center_xz.y)
            * config::HEIGHT_SCALE;

        let _ = writeln!(dump, "        {{");
        let _ = writeln!(dump, "          \"s_m\": {:.3},", section.s_m);
        dump.push_str("          \"center_world\": ");
        Self::append_vector3_literal(dump, center_world);
        dump.push_str(",\n");
        dump.push_str("          \"tangent_xz\": ");
        Self::append_vector2_literal(dump, section.tangent_xz);
        dump.push_str(",\n");
        dump.push_str("          \"lateral_xz\": ");
        Self::append_vector2_literal(dump, section.lateral_xz);
        dump.push_str(",\n");
        let _ = writeln!(
            dump,
            "          \"source_center_y_m\": {:.3},",
            source_center_y_m
        );
        let _ = writeln!(
            dump,
            "          \"visual_center_y_m\": {:.3},",
            visual_center_y_m
        );

        if let (Some(first_band), Some(last_band)) = (section.bands.first(), section.bands.last()) {
            let left_road = self.section_boundary_world_point(
                section,
                first_band.lateral_start_m,
                first_band.height_start_m,
            );
            let right_road = self.section_boundary_world_point(
                section,
                last_band.lateral_end_m,
                last_band.height_end_m,
            );
            dump.push_str("          \"left_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, left_road);
            dump.push_str(",\n");
            dump.push_str("          \"right_road_edge\": ");
            Self::append_surface_sample_literal(dump, terrain, right_road);
            dump.push_str(",\n");
            if let (Some(left_outer), Some(right_outer)) = (
                self.earthwork_transition_point(left_road, section.lateral_xz * -1.0, terrain),
                self.earthwork_transition_point(right_road, section.lateral_xz, terrain),
            ) {
                dump.push_str("          \"left_outer_margin\": ");
                Self::append_surface_sample_literal(dump, terrain, left_outer);
                dump.push_str(",\n");
                dump.push_str("          \"right_outer_margin\": ");
                Self::append_surface_sample_literal(dump, terrain, right_outer);
                dump.push_str(",\n");
            }
        }

        let _ = writeln!(dump, "          \"bands\": [");
        for (band_index, band) in section.bands.iter().enumerate() {
            if band_index > 0 {
                let _ = writeln!(dump, ",");
            }
            let _ = write!(
                dump,
                "            {{\"kind\":\"{:?}\",\"lateral_start_m\":{:.3},\"lateral_end_m\":{:.3},\"height_start_m\":{:.3},\"height_end_m\":{:.3}}}",
                band.kind,
                band.lateral_start_m,
                band.lateral_end_m,
                band.height_start_m,
                band.height_end_m
            );
        }
        let _ = writeln!(dump);
        let _ = writeln!(dump, "          ]");
        let _ = write!(dump, "        }}");
    }

    pub(in crate::simulation::network::surface::debug) fn append_span_ownership_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('{');
        let _ = write!(
            dump,
            "\"owned_region_count\":{}",
            piece.span_owned_regions.len()
        );
        for role in [
            RoadSurfaceSpanRegionRole::Asphalt,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            RoadSurfaceSpanRegionRole::NonRoad,
        ] {
            let count = piece
                .span_owned_regions
                .iter()
                .filter(|region| region.role == role)
                .count();
            let _ = write!(
                dump,
                ",\"{}\":{}",
                Self::span_region_role_debug_name(role),
                count
            );
        }
        for kind in Self::debug_band_kind_order() {
            let count = piece
                .span_owned_regions
                .iter()
                .filter(|region| region.owner.kind == kind)
                .count();
            let _ = write!(dump, ",\"band_{:?}\":{}", kind, count);
        }
        dump.push_str(",\"regions\":[");
        for (region_index, region) in piece.span_owned_regions.iter().enumerate() {
            if region_index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(
                dump,
                "{{\"edge_idx\":{},\"role\":\"{}\",\"source_band_index\":{},\"band_kind\":\"{:?}\",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3},\"point_count\":{},\"height_min_m\":",
                region.edge_idx,
                Self::span_region_role_debug_name(region.role),
                region.owner.source_band_index,
                region.owner.kind,
                region.start_section_index,
                region.end_section_index,
                region.start_s_m,
                region.end_s_m,
                region.polygon.points_world.len(),
            );
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(min_y, _)| min_y),
            );
            dump.push_str(",\"height_max_m\":");
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(_, max_y)| max_y),
            );
            dump.push('}');
        }
        dump.push_str("]}");
    }

    pub(in crate::simulation::network::surface::debug) fn append_span_earthwork_support_debug_literal(
        dump: &mut String,
        piece: &RoadSurfaceVisualSpanPiece,
    ) {
        dump.push('{');
        let _ = write!(
            dump,
            "\"support_region_count\":{},\"edge_class\":\"{:?}\",\"support_policy\":\"{}\"",
            piece.span_earthwork_support_regions.len(),
            piece.edge_class,
            RoadSurfaceEarthworkSupportPolicy::from_edge_class(piece.edge_class).debug_name()
        );
        for role in [
            RoadSurfaceSpanRegionRole::Asphalt,
            RoadSurfaceSpanRegionRole::CurbOrShoulder,
            RoadSurfaceSpanRegionRole::NonRoad,
        ] {
            let count = piece
                .span_earthwork_support_regions
                .iter()
                .filter(|region| region.role == role)
                .count();
            let _ = write!(
                dump,
                ",\"{}\":{}",
                Self::span_region_role_debug_name(role),
                count
            );
        }
        for kind in Self::debug_band_kind_order() {
            let count = piece
                .span_earthwork_support_regions
                .iter()
                .filter(|region| region.owner.kind == kind)
                .count();
            let _ = write!(dump, ",\"band_{:?}\":{}", kind, count);
        }
        dump.push_str(",\"regions\":[");
        for (region_index, region) in piece.span_earthwork_support_regions.iter().enumerate() {
            if region_index > 0 {
                dump.push_str(", ");
            }
            let _ = write!(
                dump,
                "{{\"edge_idx\":{},\"role\":\"{}\",\"source_band_index\":{},\"band_kind\":\"{:?}\",\"start_section_index\":{},\"end_section_index\":{},\"start_s_m\":{:.3},\"end_s_m\":{:.3},\"point_count\":{},\"height_min_m\":",
                region.edge_idx,
                Self::span_region_role_debug_name(region.role),
                region.owner.source_band_index,
                region.owner.kind,
                region.start_section_index,
                region.end_section_index,
                region.start_s_m,
                region.end_s_m,
                region.polygon.points_world.len(),
            );
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(min_y, _)| min_y),
            );
            dump.push_str(",\"height_max_m\":");
            Self::append_optional_f32_precise_literal(
                dump,
                Self::debug_polygon_height_range(&region.polygon).map(|(_, max_y)| max_y),
            );
            dump.push('}');
        }
        dump.push_str("]}");
    }

    pub(in crate::simulation::network::surface::debug) fn append_span_band_owner_debug_literal(
        dump: &mut String,
        owner: RoadSurfaceSpanBandOwner,
    ) {
        let _ = write!(
            dump,
            "{{\"source_band_index\":{},\"kind\":\"{:?}\"}}",
            owner.source_band_index, owner.kind
        );
    }

    pub(in crate::simulation::network::surface::debug) fn span_region_projection_matches_from_regions(
        regions: &[RoadSurfaceSpanOwnedRegion],
        role: RoadSurfaceSpanRegionRole,
        projected: &[RoadSurfaceVisualPolygon],
    ) -> bool {
        let mut expected: Vec<RoadSurfaceVisualPolygon> = regions
            .iter()
            .filter(|region| region.role == role)
            .map(|region| region.polygon.clone())
            .collect();
        let mut actual = projected.to_vec();
        Self::sort_visual_polygons(&mut expected);
        Self::sort_visual_polygons(&mut actual);
        expected == actual
    }

    pub(in crate::simulation::network::surface::debug) fn debug_polygon_height_range(
        polygon: &RoadSurfaceVisualPolygon,
    ) -> Option<(f32, f32)> {
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for point in &polygon.points_world {
            min_y = min_y.min(point.y);
            max_y = max_y.max(point.y);
        }
        min_y.is_finite().then_some((min_y, max_y))
    }

    pub(in crate::simulation::network::surface::debug) fn span_region_role_debug_name(
        role: RoadSurfaceSpanRegionRole,
    ) -> &'static str {
        match role {
            RoadSurfaceSpanRegionRole::Asphalt => "asphalt",
            RoadSurfaceSpanRegionRole::CurbOrShoulder => "curb_or_shoulder",
            RoadSurfaceSpanRegionRole::NonRoad => "non_road",
        }
    }
}
