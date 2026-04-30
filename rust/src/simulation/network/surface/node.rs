//! Explicit visual node-piece construction and incident-edge classification.

use super::{
    CompiledNodeKind, IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile,
    IncidentSurfaceEdge, NodeBandHeightDomain, NodeCorridorCandidates, NodeNonRoadCandidatePolygon,
    OrderedIncidentPieceMouth, RoadSurfaceBandKind, RoadSurfaceEarthworkRenderFace,
    RoadSurfaceSystem, RoadSurfaceVisualNodePiece, RoadSurfaceVisualNodePieceKind,
    RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M, TerminalEndBandLayer,
};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};

// Node-piece classification and bend-join arc sampling thresholds.
const PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const BEND_JOIN_ARC_SAMPLE_STEP_M: f32 = 0.75;

impl RoadSurfaceSystem {
    pub(super) fn compile_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let valid = graph.get_valid_node(node_id);
        let incidents = self.sorted_incident_surface_edges(graph, valid);
        match self.classify_visual_node_kind(&incidents) {
            CompiledNodeKind::Terminal => incidents.first().and_then(|incident| {
                self.build_terminal_visual_node_piece(terrain, valid, *incident)
            }),
            CompiledNodeKind::PassThrough => None,
            CompiledNodeKind::Bend => {
                self.build_bend_visual_node_piece(graph, terrain, valid, &incidents)
            }
            CompiledNodeKind::JunctionN => {
                self.build_junction_visual_node_piece(graph, terrain, valid, &incidents)
            }
        }
    }
    fn build_terminal_visual_node_piece(
        &self,
        terrain: &TerrainSystem,
        node_id: u32,
        incident: IncidentSurfaceEdge,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        let mouths = self.build_ordered_piece_mouths(&[incident])?;
        let mouth = mouths.first()?;
        let endpoint_profile = self.build_incident_endpoint_profile(incident)?;
        let (road_surface_polygons, sidewalk_surface_polygons) =
            Self::build_terminal_band_surface_polygons(&mouth.profile, &endpoint_profile)?;
        let mut footprint_polygons =
            Vec::with_capacity(road_surface_polygons.len() + sidewalk_surface_polygons.len());
        footprint_polygons.extend(road_surface_polygons.iter().cloned());
        footprint_polygons.extend(sidewalk_surface_polygons.iter().cloned());
        let outer_boundary_loops = Self::union_terrain_clip_polygons(&footprint_polygons);
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            RoadSurfaceVisualNodePieceKind::Terminal,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_bend_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() != 2 {
            return None;
        }
        let node_pos = graph.node(node_id).pos;
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        let node_candidates = Self::build_node_corridor_candidates(node_pos, &mouths)?;
        let node_regions = Self::resolve_node_surface_regions_with_overlay(
            node_id,
            RoadSurfaceVisualNodePieceKind::Bend,
            &node_candidates.road_candidate_polygons,
            &node_candidates.non_road_candidate_polygons,
            &node_candidates.non_road_height_domains,
        )?;
        let outer_boundary_loops = node_regions.outer_boundary_loops;
        let road_surface_polygons = node_regions.road_surface_polygons;
        let sidewalk_surface_polygons = node_regions.sidewalk_surface_polygons;
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            RoadSurfaceVisualNodePieceKind::Bend,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_junction_visual_node_piece(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        node_id: u32,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if incidents.len() < 3 {
            return None;
        }
        let node_pos = graph.node(node_id).pos;
        let mouths = self.build_ordered_piece_mouths(incidents)?;
        let node_candidates = Self::build_node_corridor_candidates(node_pos, &mouths)?;
        let node_regions = Self::resolve_node_surface_regions_with_overlay(
            node_id,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &node_candidates.road_candidate_polygons,
            &node_candidates.non_road_candidate_polygons,
            &node_candidates.non_road_height_domains,
        )?;
        let outer_boundary_loops = node_regions.outer_boundary_loops;
        let road_surface_polygons = node_regions.road_surface_polygons;
        let sidewalk_surface_polygons = node_regions.sidewalk_surface_polygons;
        let (earthwork_surface_polygons, earthwork_outer_boundary_loops, render_earthwork_faces) =
            self.build_closed_earthwork_geometry_from_boundary_loops(
                &outer_boundary_loops,
                terrain,
            );

        self.assemble_explicit_node_piece(
            node_id,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        )
    }

    fn build_ordered_piece_mouths(
        &self,
        incidents: &[IncidentSurfaceEdge],
    ) -> Option<Vec<OrderedIncidentPieceMouth>> {
        let mut mouths = Vec::with_capacity(incidents.len());
        for &incident in incidents {
            mouths.push(OrderedIncidentPieceMouth {
                profile: self.build_incident_mouth_profile(incident)?,
                direction_angle_ccw: Self::normalized_angle_ccw(incident.direction_xz),
                direction_xz: incident.direction_xz,
                edge_idx: incident.edge_idx,
                side: incident.side,
            });
        }
        mouths.sort_by(|a, b| {
            a.direction_angle_ccw
                .total_cmp(&b.direction_angle_ccw)
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        Some(mouths)
    }

    fn build_incident_mouth_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let piece = self.compiled_visual_span_pieces.get(&incident.edge_idx)?;
        match incident.side {
            IncidentEdgeSide::Start => piece.start_mouth_profile.clone(),
            IncidentEdgeSide::End => piece.end_mouth_profile.clone(),
        }
    }

    fn build_incident_endpoint_profile(
        &self,
        incident: IncidentSurfaceEdge,
    ) -> Option<IncidentMouthProfile> {
        let sections = self.compiled_sections.get(&incident.edge_idx)?;
        let section = match incident.side {
            IncidentEdgeSide::Start => sections.first()?,
            IncidentEdgeSide::End => sections.last()?,
        };
        Self::build_mouth_profile_from_section(section, incident.side)
    }

    fn build_terminal_band_surface_polygons(
        mouth_profile: &IncidentMouthProfile,
        endpoint_profile: &IncidentMouthProfile,
    ) -> Option<(Vec<RoadSurfaceVisualPolygon>, Vec<RoadSurfaceVisualPolygon>)> {
        let mut road_surface_polygons = Vec::new();
        let mut sidewalk_surface_polygons = Vec::new();

        for (mouth_band, endpoint_band) in mouth_profile.bands.iter().zip(&endpoint_profile.bands) {
            if mouth_band.kind != endpoint_band.kind {
                continue;
            }
            let Some(polygon) = Self::make_visual_polygon(vec![
                mouth_band.start_point_world,
                mouth_band.end_point_world,
                endpoint_band.end_point_world,
                endpoint_band.start_point_world,
            ]) else {
                continue;
            };
            if mouth_band.kind == RoadSurfaceBandKind::Carriageway {
                road_surface_polygons.push(polygon);
            } else {
                sidewalk_surface_polygons.push(polygon);
            }
        }

        Self::append_terminal_end_band_polygons(endpoint_profile, &mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        (!road_surface_polygons.is_empty() || !sidewalk_surface_polygons.is_empty())
            .then_some((road_surface_polygons, sidewalk_surface_polygons))
    }

    fn append_terminal_end_band_polygons(
        endpoint_profile: &IncidentMouthProfile,
        sidewalk_surface_polygons: &mut Vec<RoadSurfaceVisualPolygon>,
    ) {
        let Some(outward_direction_xz) = Self::terminal_outward_direction_xz(endpoint_profile)
        else {
            return;
        };
        for layer in Self::terminal_end_band_layers(endpoint_profile) {
            let inner_left = Self::offset_world_point_xz(
                layer.left_inner_point_world,
                outward_direction_xz,
                layer.inner_offset_m,
            );
            let inner_right = Self::offset_world_point_xz(
                layer.right_inner_point_world,
                outward_direction_xz,
                layer.inner_offset_m,
            );
            let outer_right = Self::offset_world_point_xz(
                layer.right_outer_point_world,
                outward_direction_xz,
                layer.outer_offset_m,
            );
            let outer_left = Self::offset_world_point_xz(
                layer.left_outer_point_world,
                outward_direction_xz,
                layer.outer_offset_m,
            );

            for points in [
                vec![inner_left, inner_right, outer_right, outer_left],
                vec![
                    layer.left_outer_point_world,
                    layer.left_inner_point_world,
                    inner_left,
                    outer_left,
                ],
                vec![
                    layer.right_inner_point_world,
                    layer.right_outer_point_world,
                    outer_right,
                    inner_right,
                ],
            ] {
                let Some(polygon) = Self::make_visual_polygon(points) else {
                    continue;
                };
                sidewalk_surface_polygons.push(polygon);
            }
        }
    }

    fn terminal_end_band_layers(
        endpoint_profile: &IncidentMouthProfile,
    ) -> Vec<TerminalEndBandLayer> {
        let mut carriageway_indices =
            endpoint_profile
                .bands
                .iter()
                .enumerate()
                .filter_map(|(index, band)| {
                    (band.kind == RoadSurfaceBandKind::Carriageway).then_some(index)
                });
        let Some(first_carriageway) = carriageway_indices.next() else {
            return Vec::new();
        };
        let last_carriageway = carriageway_indices.last().unwrap_or(first_carriageway);
        let left_bands = &endpoint_profile.bands[..first_carriageway];
        let right_bands = &endpoint_profile.bands[last_carriageway + 1..];
        let mut layers = Vec::new();
        let mut inner_offset_m = 0.0;
        for (left_band, right_band) in left_bands.iter().rev().zip(right_bands.iter()) {
            if left_band.kind != right_band.kind
                || left_band.kind == RoadSurfaceBandKind::Carriageway
            {
                break;
            }
            let left_inner = left_band.end_point_world;
            let left_outer = left_band.start_point_world;
            let right_inner = right_band.start_point_world;
            let right_outer = right_band.end_point_world;
            let band_depth_m = (Self::distance_xz(left_inner, left_outer)
                + Self::distance_xz(right_inner, right_outer))
                * 0.5;
            if band_depth_m <= SAMPLE_EPSILON_M {
                continue;
            }
            let outer_offset_m = inner_offset_m + band_depth_m;
            layers.push(TerminalEndBandLayer {
                left_inner_point_world: left_inner,
                left_outer_point_world: left_outer,
                right_inner_point_world: right_inner,
                right_outer_point_world: right_outer,
                inner_offset_m,
                outer_offset_m,
            });
            inner_offset_m = outer_offset_m;
        }
        layers
    }

    fn terminal_outward_direction_xz(profile: &IncidentMouthProfile) -> Option<Vector2> {
        let outward_direction_xz = -profile.inward_direction_xz;
        (outward_direction_xz.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
            .then_some(outward_direction_xz.normalized())
    }

    fn distance_xz(a: Vector3, b: Vector3) -> f32 {
        Vector2::new(b.x - a.x, b.z - a.z).length()
    }

    fn offset_world_point_xz(point: Vector3, direction_xz: Vector2, distance_m: f32) -> Vector3 {
        Vector3::new(
            point.x + direction_xz.x * distance_m,
            point.y,
            point.z + direction_xz.y * distance_m,
        )
    }

    fn build_node_corridor_candidates(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<NodeCorridorCandidates> {
        if mouths.len() == 2 {
            return Self::build_bend_corridor_candidates(node_pos, mouths);
        }

        let mut road_candidate_polygons = Vec::new();
        let mut non_road_candidate_polygons = Vec::new();
        let mut non_road_height_domains = Vec::new();

        for mouth in mouths {
            let Some((outer_a, outer_b)) = Self::mouth_full_roadbed_segment(&mouth.profile) else {
                continue;
            };
            if let Some(polygon) =
                Self::build_mouth_corridor_polygon(node_pos, mouth.direction_xz, outer_a, outer_b)
            {
                non_road_candidate_polygons.push(NodeNonRoadCandidatePolygon { polygon });
            }

            if let Some((carriageway_a, carriageway_b)) =
                Self::mouth_carriageway_segment(&mouth.profile)
            {
                if let Some(polygon) = Self::build_mouth_corridor_polygon(
                    node_pos,
                    mouth.direction_xz,
                    carriageway_a,
                    carriageway_b,
                ) {
                    road_candidate_polygons.push(polygon);
                }
            }

            Self::append_mouth_non_road_height_candidates(
                node_pos,
                mouth,
                &mut non_road_height_domains,
            );
        }
        Self::append_adjacent_non_road_height_join_domains(
            node_pos,
            mouths,
            &mut non_road_height_domains,
        );

        (!road_candidate_polygons.is_empty() || !non_road_candidate_polygons.is_empty()).then_some(
            NodeCorridorCandidates {
                road_candidate_polygons,
                non_road_candidate_polygons,
                non_road_height_domains,
            },
        )
    }

    fn build_bend_corridor_candidates(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
    ) -> Option<NodeCorridorCandidates> {
        let Some((start_index, end_index)) = Self::bend_join_mouth_order(mouths) else {
            return None;
        };
        let start_mouth = &mouths[start_index];
        let end_mouth = &mouths[end_index];
        let mut road_candidate_polygons = Vec::new();
        let mut non_road_candidate_polygons = Vec::new();
        let mut non_road_height_domains = Vec::new();

        // Keep each bend corridor/join as a simple overlay candidate; merging them into one loop
        // can make the two throat caps cross at the node on tight bends.
        for mouth in [start_mouth, end_mouth] {
            if let Some((outer_a, outer_b)) = Self::mouth_full_roadbed_segment(&mouth.profile) {
                if let Some(polygon) = Self::build_mouth_corridor_polygon(
                    node_pos,
                    mouth.direction_xz,
                    outer_a,
                    outer_b,
                ) {
                    non_road_candidate_polygons.push(NodeNonRoadCandidatePolygon { polygon });
                }
            }

            if let Some((carriageway_a, carriageway_b)) =
                Self::mouth_carriageway_segment(&mouth.profile)
            {
                if let Some(polygon) = Self::build_mouth_corridor_polygon(
                    node_pos,
                    mouth.direction_xz,
                    carriageway_a,
                    carriageway_b,
                ) {
                    road_candidate_polygons.push(polygon);
                }
            }

            Self::append_mouth_non_road_height_candidates(
                node_pos,
                mouth,
                &mut non_road_height_domains,
            );
        }

        for left_side in [true, false] {
            if let Some(polygon) = Self::build_bend_local_side_join_polygon(
                node_pos,
                start_mouth,
                end_mouth,
                Self::mouth_full_roadbed_segment,
                left_side,
            ) {
                non_road_candidate_polygons.push(NodeNonRoadCandidatePolygon { polygon });
            }

            if let Some(polygon) = Self::build_bend_local_side_join_polygon(
                node_pos,
                start_mouth,
                end_mouth,
                Self::mouth_carriageway_segment,
                left_side,
            ) {
                road_candidate_polygons.push(polygon);
            }
        }

        for (start_band, end_band) in start_mouth
            .profile
            .bands
            .iter()
            .zip(&end_mouth.profile.bands)
        {
            if start_band.kind != end_band.kind
                || start_band.kind == RoadSurfaceBandKind::Carriageway
            {
                continue;
            }
            if let Some(polygon) = Self::build_bend_local_band_join_polygon(
                node_pos,
                start_mouth,
                end_mouth,
                start_band,
                end_band,
            ) {
                non_road_height_domains.push(NodeBandHeightDomain {
                    kind: start_band.kind,
                    polygon,
                });
            }
        }

        (!road_candidate_polygons.is_empty() || !non_road_candidate_polygons.is_empty()).then_some(
            NodeCorridorCandidates {
                road_candidate_polygons,
                non_road_candidate_polygons,
                non_road_height_domains,
            },
        )
    }

    fn append_mouth_non_road_height_candidates(
        node_pos: Vector3,
        mouth: &OrderedIncidentPieceMouth,
        non_road_height_domains: &mut Vec<NodeBandHeightDomain>,
    ) {
        for band in &mouth.profile.bands {
            if band.kind == RoadSurfaceBandKind::Carriageway {
                continue;
            }
            let Some(polygon) = Self::build_mouth_corridor_polygon(
                node_pos,
                mouth.direction_xz,
                band.start_point_world,
                band.end_point_world,
            ) else {
                continue;
            };
            non_road_height_domains.push(NodeBandHeightDomain {
                kind: band.kind,
                polygon,
            });
        }
    }

    fn append_adjacent_non_road_height_join_domains(
        node_pos: Vector3,
        mouths: &[OrderedIncidentPieceMouth],
        non_road_height_domains: &mut Vec<NodeBandHeightDomain>,
    ) {
        if mouths.len() < 2 {
            return;
        }

        for index in 0..mouths.len() {
            let start_mouth = &mouths[index];
            let end_mouth = &mouths[(index + 1) % mouths.len()];
            for (start_band, end_band) in start_mouth
                .profile
                .bands
                .iter()
                .zip(&end_mouth.profile.bands)
            {
                if start_band.kind != end_band.kind
                    || start_band.kind == RoadSurfaceBandKind::Carriageway
                {
                    continue;
                }
                if let Some(polygon) = Self::build_bend_local_band_join_polygon(
                    node_pos,
                    start_mouth,
                    end_mouth,
                    start_band,
                    end_band,
                ) {
                    non_road_height_domains.push(NodeBandHeightDomain {
                        kind: start_band.kind,
                        polygon,
                    });
                }
            }
        }
    }

    fn mouth_full_roadbed_segment(profile: &IncidentMouthProfile) -> Option<(Vector3, Vector3)> {
        Some((
            *profile.boundary_points_world.first()?,
            *profile.boundary_points_world.last()?,
        ))
    }

    fn mouth_carriageway_segment(profile: &IncidentMouthProfile) -> Option<(Vector3, Vector3)> {
        let mut carriageway_indices =
            profile
                .bands
                .iter()
                .enumerate()
                .filter_map(|(index, band)| {
                    (band.kind == RoadSurfaceBandKind::Carriageway).then_some(index)
                });
        let first_carriageway = carriageway_indices.next()?;
        let last_carriageway = carriageway_indices.last().unwrap_or(first_carriageway);
        Some((
            *profile.boundary_points_world.get(first_carriageway)?,
            *profile.boundary_points_world.get(last_carriageway + 1)?,
        ))
    }

    fn bend_join_mouth_order(mouths: &[OrderedIncidentPieceMouth]) -> Option<(usize, usize)> {
        if mouths.len() != 2 {
            return None;
        }
        let angle_a = mouths[0].direction_angle_ccw;
        let angle_b = mouths[1].direction_angle_ccw;
        let diff_ab = (angle_b - angle_a).rem_euclid(std::f32::consts::TAU);
        if diff_ab <= SAMPLE_EPSILON_M {
            return None;
        }
        if diff_ab <= std::f32::consts::PI {
            Some((0, 1))
        } else {
            Some((1, 0))
        }
    }

    fn build_bend_local_side_join_polygon(
        node_pos: Vector3,
        start_mouth: &OrderedIncidentPieceMouth,
        end_mouth: &OrderedIncidentPieceMouth,
        segment_fn: fn(&IncidentMouthProfile) -> Option<(Vector3, Vector3)>,
        left_side: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let (start_a, start_b) = segment_fn(&start_mouth.profile)?;
        let (end_a, end_b) = segment_fn(&end_mouth.profile)?;
        let start_travel = -start_mouth.direction_xz;
        let end_travel = end_mouth.direction_xz;
        if start_travel.length_squared() <= SAMPLE_EPSILON_M
            || end_travel.length_squared() <= SAMPLE_EPSILON_M
        {
            return None;
        }
        let start_travel = start_travel.normalized();
        let end_travel = end_travel.normalized();
        let turn = Self::cross_xz(start_travel, end_travel);
        if turn.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let (start_left, start_right) =
            Self::segment_left_right_for_travel(start_travel, start_a, start_b)?;
        let (end_left, end_right) = Self::segment_left_right_for_travel(end_travel, end_a, end_b)?;
        let start_center = Self::midpoint_world(start_left, start_right);
        let end_center = Self::midpoint_world(end_left, end_right);
        let (start_side, end_side) = if left_side {
            (start_left, end_left)
        } else {
            (start_right, end_right)
        };
        let start_node =
            Self::bend_node_side_point(node_pos, start_travel, start_center, start_side, left_side);
        let end_node =
            Self::bend_node_side_point(node_pos, end_travel, end_center, end_side, left_side);
        let ccw = Self::bend_short_arc_is_ccw(node_pos, start_node, end_node)?;
        let center_height = (start_node.y + end_node.y) * 0.5;
        let mut points_world = vec![
            Vector3::new(node_pos.x, center_height, node_pos.z),
            start_node,
        ];
        Self::append_bend_arc_points(&mut points_world, node_pos, start_node, end_node, ccw);
        Self::make_visual_polygon(points_world)
    }

    fn build_bend_local_band_join_polygon(
        node_pos: Vector3,
        start_mouth: &OrderedIncidentPieceMouth,
        end_mouth: &OrderedIncidentPieceMouth,
        start_band: &IncidentMouthBand,
        end_band: &IncidentMouthBand,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let start_a = start_band.start_point_world;
        let start_b = start_band.end_point_world;
        let end_a = end_band.start_point_world;
        let end_b = end_band.end_point_world;
        Self::build_bend_local_segment_join_polygon(
            node_pos,
            start_mouth.direction_xz,
            end_mouth.direction_xz,
            start_a,
            start_b,
            end_a,
            end_b,
        )
    }

    fn build_bend_local_segment_join_polygon(
        node_pos: Vector3,
        start_direction_xz: Vector2,
        end_direction_xz: Vector2,
        start_a: Vector3,
        start_b: Vector3,
        end_a: Vector3,
        end_b: Vector3,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let start_travel = -start_direction_xz;
        let end_travel = end_direction_xz;
        if start_travel.length_squared() <= SAMPLE_EPSILON_M
            || end_travel.length_squared() <= SAMPLE_EPSILON_M
        {
            return None;
        }
        let start_travel = start_travel.normalized();
        let end_travel = end_travel.normalized();
        let turn = Self::cross_xz(start_travel, end_travel);
        if turn.abs() <= SAMPLE_EPSILON_M {
            return None;
        }

        let (start_left, start_right) =
            Self::segment_left_right_for_travel(start_travel, start_a, start_b)?;
        let (end_left, end_right) = Self::segment_left_right_for_travel(end_travel, end_a, end_b)?;
        let start_center = Self::midpoint_world(start_left, start_right);
        let end_center = Self::midpoint_world(end_left, end_right);
        let start_left_node =
            Self::bend_node_side_point(node_pos, start_travel, start_center, start_left, true);
        let start_right_node =
            Self::bend_node_side_point(node_pos, start_travel, start_center, start_right, false);
        let end_left_node =
            Self::bend_node_side_point(node_pos, end_travel, end_center, end_left, true);
        let end_right_node =
            Self::bend_node_side_point(node_pos, end_travel, end_center, end_right, false);
        let ccw = Self::bend_short_arc_is_ccw(node_pos, start_left_node, end_left_node)?;
        let mut points_world = vec![start_left_node];
        Self::append_bend_arc_points(
            &mut points_world,
            node_pos,
            start_left_node,
            end_left_node,
            ccw,
        );
        points_world.push(end_right_node);
        Self::append_bend_arc_points(
            &mut points_world,
            node_pos,
            end_right_node,
            start_right_node,
            !ccw,
        );
        Self::make_visual_polygon(points_world)
    }

    fn bend_short_arc_is_ccw(node_pos: Vector3, from: Vector3, to: Vector3) -> Option<bool> {
        let from_vector = Vector2::new(from.x - node_pos.x, from.z - node_pos.z);
        let to_vector = Vector2::new(to.x - node_pos.x, to.z - node_pos.z);
        if from_vector.length_squared() <= SAMPLE_EPSILON_M
            || to_vector.length_squared() <= SAMPLE_EPSILON_M
        {
            return None;
        }
        let from_angle = Self::normalized_angle_ccw(from_vector);
        let to_angle = Self::normalized_angle_ccw(to_vector);
        let ccw_span = (to_angle - from_angle).rem_euclid(std::f32::consts::TAU);
        Some(ccw_span <= std::f32::consts::PI)
    }

    fn segment_left_right_for_travel(
        travel_xz: Vector2,
        a: Vector3,
        b: Vector3,
    ) -> Option<(Vector3, Vector3)> {
        if travel_xz.length_squared() <= SAMPLE_EPSILON_M {
            return None;
        }
        let center = Vector2::new((a.x + b.x) * 0.5, (a.z + b.z) * 0.5);
        let cross_a = Self::cross_xz(travel_xz, Vector2::new(a.x, a.z) - center);
        let cross_b = Self::cross_xz(travel_xz, Vector2::new(b.x, b.z) - center);
        if cross_a >= cross_b {
            Some((a, b))
        } else {
            Some((b, a))
        }
    }

    fn bend_node_side_point(
        node_pos: Vector3,
        travel_xz: Vector2,
        segment_center: Vector3,
        side_point: Vector3,
        left_side: bool,
    ) -> Vector3 {
        let left_normal = Self::left_normal_xz(travel_xz);
        let side_normal = if left_side { left_normal } else { -left_normal };
        let side_width = Vector2::new(
            side_point.x - segment_center.x,
            side_point.z - segment_center.z,
        )
        .length();
        Vector3::new(
            node_pos.x + side_normal.x * side_width,
            side_point.y,
            node_pos.z + side_normal.y * side_width,
        )
    }

    fn append_bend_arc_points(
        points_world: &mut Vec<Vector3>,
        node_pos: Vector3,
        from: Vector3,
        to: Vector3,
        ccw: bool,
    ) {
        let from_vector = Vector2::new(from.x - node_pos.x, from.z - node_pos.z);
        let to_vector = Vector2::new(to.x - node_pos.x, to.z - node_pos.z);
        let from_radius = from_vector.length();
        let to_radius = to_vector.length();
        if from_radius <= SAMPLE_EPSILON_M || to_radius <= SAMPLE_EPSILON_M {
            return;
        }
        if points_world.last().is_none_or(|point| {
            (*point - from).length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M
        }) {
            points_world.push(from);
        }
        let from_angle = Self::normalized_angle_ccw(from_vector);
        let to_angle = Self::normalized_angle_ccw(to_vector);
        let angle_span = if ccw {
            (to_angle - from_angle).rem_euclid(std::f32::consts::TAU)
        } else {
            (from_angle - to_angle).rem_euclid(std::f32::consts::TAU)
        };
        if angle_span <= SAMPLE_EPSILON_M || angle_span > std::f32::consts::PI {
            points_world.push(to);
            return;
        }
        let max_radius = from_radius.max(to_radius);
        let segment_count = ((angle_span * max_radius) / BEND_JOIN_ARC_SAMPLE_STEP_M)
            .ceil()
            .clamp(2.0, 96.0) as usize;
        for index in 1..=segment_count {
            let t = index as f32 / segment_count as f32;
            if index == segment_count {
                points_world.push(to);
                continue;
            }
            let angle = if ccw {
                from_angle + angle_span * t
            } else {
                from_angle - angle_span * t
            };
            let radius = from_radius + (to_radius - from_radius) * t;
            let height = from.y + (to.y - from.y) * t;
            points_world.push(Vector3::new(
                node_pos.x + angle.cos() * radius,
                height,
                node_pos.z + angle.sin() * radius,
            ));
        }
    }

    fn midpoint_world(a: Vector3, b: Vector3) -> Vector3 {
        Vector3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5)
    }

    pub(super) fn left_normal_xz(direction_xz: Vector2) -> Vector2 {
        Vector2::new(-direction_xz.y, direction_xz.x)
    }

    pub(super) fn cross_xz(a: Vector2, b: Vector2) -> f32 {
        a.x * b.y - a.y * b.x
    }

    fn build_mouth_corridor_polygon(
        node_pos: Vector3,
        direction_xz: Vector2,
        segment_a: Vector3,
        segment_b: Vector3,
    ) -> Option<RoadSurfaceVisualPolygon> {
        if direction_xz.length_squared() <= SAMPLE_EPSILON_M {
            return None;
        }
        let direction_xz = direction_xz.normalized();
        let node_xz = Vector2::new(node_pos.x, node_pos.z);
        let segment_center_xz = Vector2::new(
            (segment_a.x + segment_b.x) * 0.5,
            (segment_a.z + segment_b.z) * 0.5,
        );
        let mut depth_m = (segment_center_xz - node_xz).dot(direction_xz).max(0.0);
        if depth_m <= SAMPLE_EPSILON_M {
            depth_m = Vector2::new(segment_a.x - node_pos.x, segment_a.z - node_pos.z)
                .length()
                .max(Vector2::new(segment_b.x - node_pos.x, segment_b.z - node_pos.z).length());
        }
        if depth_m <= SAMPLE_EPSILON_M {
            return None;
        }

        let backtrack = Vector3::new(direction_xz.x * depth_m, 0.0, direction_xz.y * depth_m);
        let node_a = segment_a - backtrack;
        let node_b = segment_b - backtrack;
        Self::make_visual_polygon(vec![segment_a, segment_b, node_b, node_a])
    }

    fn normalized_angle_ccw(direction_xz: Vector2) -> f32 {
        let angle = direction_xz.y.atan2(direction_xz.x);
        if angle < 0.0 {
            angle + std::f32::consts::TAU
        } else {
            angle
        }
    }

    fn assemble_explicit_node_piece(
        &self,
        node_id: u32,
        kind: RoadSurfaceVisualNodePieceKind,
        outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
        mut earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
        mut render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
    ) -> Option<RoadSurfaceVisualNodePiece> {
        if road_surface_polygons.is_empty() && sidewalk_surface_polygons.is_empty() {
            return None;
        }
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_surface_polygons);
        Self::sort_visual_polygons(&mut earthwork_outer_boundary_loops);
        Self::sort_earthwork_render_faces(&mut render_earthwork_faces);
        if outer_boundary_loops.is_empty() {
            return None;
        }
        Some(RoadSurfaceVisualNodePiece {
            node_id,
            kind,
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            earthwork_surface_polygons,
            earthwork_outer_boundary_loops,
            render_earthwork_faces,
        })
    }
    fn classify_visual_node_kind(&self, incidents: &[IncidentSurfaceEdge]) -> CompiledNodeKind {
        match incidents.len() {
            0 | 1 => CompiledNodeKind::Terminal,
            2 => {
                let a = incidents[0];
                let b = incidents[1];
                let straight = a.direction_xz.dot(b.direction_xz) <= -PASS_THROUGH_DOT_THRESHOLD;
                if !straight {
                    return CompiledNodeKind::Bend;
                }
                CompiledNodeKind::PassThrough
            }
            _ => CompiledNodeKind::JunctionN,
        }
    }

    pub(super) fn classify_surface_node_kind_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Option<CompiledNodeKind> {
        let incidents = self.sorted_incident_surface_edges_from_graph_geometry(graph, node_id);
        (!incidents.is_empty()).then(|| self.classify_visual_node_kind(&incidents))
    }

    fn sorted_incident_surface_edges_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges_from_graph_geometry(graph, node_id);
        incidents.sort_by(|a, b| {
            Self::normalized_angle_ccw(a.direction_xz)
                .total_cmp(&Self::normalized_angle_ccw(b.direction_xz))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        incidents
    }

    fn collect_incident_surface_edges_from_graph_geometry(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }

        let mut incidents = Vec::new();
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let side = if graph.get_valid_node(edge.start_node) == node_id {
                Some(IncidentEdgeSide::Start)
            } else if graph.get_valid_node(edge.end_node) == node_id {
                Some(IncidentEdgeSide::End)
            } else {
                None
            };
            let Some(side) = side else {
                continue;
            };
            let Some(direction_xz) = self.incident_direction_from_edge_geometry(edge, side) else {
                continue;
            };
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz,
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }

    fn incident_direction_from_edge_geometry(
        &self,
        edge: &Edge,
        side: IncidentEdgeSide,
    ) -> Option<Vector2> {
        let points = self.edge_points(edge);
        if points.len() < 2 {
            return None;
        }

        match side {
            IncidentEdgeSide::Start => {
                let endpoint = points[0];
                points.iter().skip(1).find_map(|point| {
                    let direction = Vector2::new(point.x - endpoint.x, point.z - endpoint.z);
                    (direction.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
                        .then(|| direction.normalized())
                })
            }
            IncidentEdgeSide::End => {
                let endpoint = *points.last()?;
                points.iter().rev().skip(1).find_map(|point| {
                    let direction = Vector2::new(point.x - endpoint.x, point.z - endpoint.z);
                    (direction.length_squared() > SAMPLE_EPSILON_M * SAMPLE_EPSILON_M)
                        .then(|| direction.normalized())
                })
            }
        }
    }

    fn sorted_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        let mut incidents = self.collect_incident_surface_edges(graph, node_id);
        incidents.sort_by(|a, b| {
            Self::normalized_angle_ccw(a.direction_xz)
                .total_cmp(&Self::normalized_angle_ccw(b.direction_xz))
                .then(a.edge_idx.cmp(&b.edge_idx))
                .then(a.side.cmp(&b.side))
        });
        incidents
    }

    fn collect_incident_surface_edges(
        &self,
        graph: &RegionGraph,
        node_id: u32,
    ) -> Vec<IncidentSurfaceEdge> {
        if node_id as usize >= graph.node_adjacency_count() {
            return Vec::new();
        }

        let mut incidents = Vec::new();
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if !Self::is_surface_edge(edge) {
                continue;
            }

            let side = if graph.get_valid_node(edge.start_node) == node_id {
                Some(IncidentEdgeSide::Start)
            } else if graph.get_valid_node(edge.end_node) == node_id {
                Some(IncidentEdgeSide::End)
            } else {
                None
            };
            let Some(side) = side else {
                continue;
            };
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let Some(direction_xz) = (match side {
                IncidentEdgeSide::Start => piece
                    .start_mouth_profile
                    .as_ref()
                    .map(|mouth| mouth.inward_direction_xz),
                IncidentEdgeSide::End => piece
                    .end_mouth_profile
                    .as_ref()
                    .map(|mouth| mouth.inward_direction_xz),
            }) else {
                continue;
            };
            incidents.push(IncidentSurfaceEdge {
                edge_idx,
                side,
                direction_xz,
            });
        }

        incidents.sort_by(|a, b| a.edge_idx.cmp(&b.edge_idx).then(a.side.cmp(&b.side)));
        incidents
    }
}
