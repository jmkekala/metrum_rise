//! Overlay boolean geometry and node-region reconstruction for road surfaces.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, NodeGradeCarrier, NodeNonRoadCandidatePolygon, NodeOverlayContour,
    NodeOverlayPoint, NodeOverlayPointKey, NodeOverlayShape, NodeOverlayShapes, NodeOwnedRegion,
    NodeSurfaceRegionResult, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, SAMPLE_EPSILON_M, SurfaceCdt,
    WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
};
use godot::prelude::{Vector2, Vector3};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::scale::FixedScaleFloatOverlay;
use i_overlay::float::simplify::SimplifyShape;
use spade::{Point2, Triangulation};
use std::collections::{BTreeMap, BTreeSet};

// Overlay boolean operations quantize coordinates to millimetres for deterministic keys.
const NODE_OVERLAY_SCALE: f32 = 1000.0;
const NODE_SURFACE_HEIGHT_EPSILON_M: f32 = 1.0 / NODE_OVERLAY_SCALE;
const NODE_SURFACE_SHARED_SEAM_WELD_MAX_SLOPE_RATIO: f32 = 2.0;

impl RoadSurfaceSystem {
    pub(super) fn resolve_node_surface_regions_with_overlay(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        road_candidates: &[RoadSurfaceVisualPolygon],
        non_road_candidates: &[NodeNonRoadCandidatePolygon],
        non_road_height_domains: &[NodeGradeCarrier],
    ) -> Option<NodeSurfaceRegionResult> {
        let mut height_candidates = Vec::with_capacity(
            road_candidates
                .len()
                .saturating_add(non_road_candidates.len()),
        );
        height_candidates.extend(road_candidates.iter().cloned());
        height_candidates.extend(
            non_road_candidates
                .iter()
                .map(|candidate| candidate.polygon.clone()),
        );
        let road_height_domains = road_candidates
            .iter()
            .cloned()
            .map(|polygon| NodeGradeCarrier {
                kind: RoadSurfaceBandKind::Carriageway,
                polygon,
            })
            .collect::<Vec<_>>();

        let road_contours = Self::overlay_contours_from_polygons(road_candidates);
        let mut road_shapes = Self::overlay_union_contours(&road_contours)?;

        let footprint_contours = Self::overlay_contours_from_polygons(&height_candidates);
        let mut footprint_shapes = Self::overlay_union_contours(&footprint_contours)?;

        let mut non_road_shapes = if road_shapes.is_empty() {
            footprint_shapes.clone()
        } else if footprint_shapes.is_empty() {
            Vec::new()
        } else {
            Self::overlay_binary_shapes(&footprint_shapes, &road_shapes, OverlayRule::Difference)?
        };
        Self::sort_overlay_shapes(&mut road_shapes);
        Self::sort_overlay_shapes(&mut non_road_shapes);
        Self::sort_overlay_shapes(&mut footprint_shapes);
        let resolved_non_road_height_domains = non_road_height_domains.to_vec();

        let mut owned_regions =
            Vec::with_capacity(road_height_domains.len() + resolved_non_road_height_domains.len());
        owned_regions.extend(Self::owned_regions_from_height_domains(
            node_id,
            piece_kind,
            "carriageway",
            &road_shapes,
            &road_height_domains,
        )?);
        owned_regions.extend(Self::owned_non_road_regions_from_height_domains(
            node_id,
            piece_kind,
            &non_road_shapes,
            &road_shapes,
            &resolved_non_road_height_domains,
        )?);
        if owned_regions.is_empty() {
            return None;
        }
        Self::split_owned_region_contours_at_shared_vertices(&mut owned_regions)?;
        if !Self::normalize_owned_region_surface_heights(&mut owned_regions) {
            return None;
        }
        Self::insert_outer_boundary_vertices_into_owned_regions(&mut owned_regions)?;
        if !Self::normalize_owned_region_surface_heights(&mut owned_regions) {
            return None;
        }
        Self::weld_shared_top_surface_edges(&mut owned_regions);
        Self::sort_node_owned_regions(&mut owned_regions);

        let road_surface_polygons = owned_regions
            .iter()
            .filter(|region| region.kind == RoadSurfaceBandKind::Carriageway)
            .map(|region| region.polygon.clone())
            .collect::<Vec<_>>();
        let sidewalk_surface_polygons = owned_regions
            .iter()
            .filter(|region| region.kind != RoadSurfaceBandKind::Carriageway)
            .map(|region| region.polygon.clone())
            .collect::<Vec<_>>();
        let mut outer_boundary_loops = Self::outer_boundary_loops_from_canonical_owned_regions(
            node_id,
            piece_kind,
            &owned_regions,
        )?;
        if outer_boundary_loops.is_empty() {
            return None;
        }

        let mut road_surface_polygons = road_surface_polygons;
        let mut sidewalk_surface_polygons = sidewalk_surface_polygons;
        Self::sort_visual_polygons(&mut road_surface_polygons);
        Self::sort_visual_polygons(&mut sidewalk_surface_polygons);
        Self::sort_visual_polygons(&mut outer_boundary_loops);
        Some(NodeSurfaceRegionResult {
            outer_boundary_loops,
            road_surface_polygons,
            sidewalk_surface_polygons,
            owned_regions,
        })
    }

    pub(super) fn outer_boundary_loops_from_canonical_owned_regions(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        owned_regions: &[NodeOwnedRegion],
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        let visible_surface_polygons = owned_regions
            .iter()
            .map(|region| region.polygon.clone())
            .collect::<Vec<_>>();
        let mut loops = Self::union_terrain_clip_polygons(&visible_surface_polygons);
        if loops.is_empty() {
            crate::debug_log!(
                "road",
                "node_boundary_no_canonical_loops node={} piece={:?}",
                node_id,
                piece_kind
            );
            return None;
        }
        if !Self::snap_outer_boundary_loop_heights_to_owned_top_vertices(
            node_id,
            piece_kind,
            &mut loops,
            owned_regions,
        ) {
            crate::debug_log!(
                "road",
                "node_boundary_missing_top_vertices node={} piece={:?}",
                node_id,
                piece_kind
            );
            return None;
        }
        Self::sort_visual_polygons(&mut loops);
        Some(loops)
    }

    pub(super) fn snap_outer_boundary_loop_heights_to_owned_top_vertices(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        loops: &mut Vec<RoadSurfaceVisualPolygon>,
        owned_regions: &[NodeOwnedRegion],
    ) -> bool {
        let mut heights_by_key: BTreeMap<NodeOverlayPointKey, Vec<f32>> = BTreeMap::new();
        for point in owned_regions
            .iter()
            .map(|region| &region.polygon)
            .flat_map(|polygon| {
                polygon.points_world.iter().copied().chain(
                    polygon
                        .triangles_world
                        .iter()
                        .flat_map(|triangle| triangle.iter().copied()),
                )
            })
        {
            heights_by_key
                .entry(Self::overlay_point_key([point.x, point.z]))
                .or_default()
                .push(point.y);
        }

        for heights in heights_by_key.values_mut() {
            heights.sort_by(|a, b| a.total_cmp(b));
        }

        let visible_surface_polygons = owned_regions
            .iter()
            .map(|region| region.polygon.clone())
            .collect::<Vec<_>>();
        let mut snapped_loops = Vec::with_capacity(loops.len());
        for loop_polygon in loops.iter() {
            let mut snapped_points = Vec::with_capacity(loop_polygon.points_world.len());
            for point in &loop_polygon.points_world {
                let key = Self::overlay_point_key([point.x, point.z]);
                let height = if let Some(heights) = heights_by_key.get(&key) {
                    if let Some(height) = Self::canonical_height_sample(heights.iter().copied()) {
                        height
                    } else {
                        let Some(height) = Self::canonical_height_sample_for_reference(
                            heights.iter().copied(),
                            point.y,
                        ) else {
                            crate::debug_log!(
                                "road",
                                "node_boundary_ambiguous_top_height node={} piece={:?} x={:.3} z={:.3}",
                                node_id,
                                piece_kind,
                                point.x,
                                point.z
                            );
                            return false;
                        };
                        height
                    }
                } else {
                    let Some(height) = Self::sample_height_from_candidate_coverage(
                        Vector2::new(point.x, point.z),
                        &visible_surface_polygons,
                    ) else {
                        return false;
                    };
                    crate::debug_log!(
                        "road",
                        "node_boundary_sampled_missing_top_vertex node={} piece={:?} x={:.3} z={:.3} y={:.3}",
                        node_id,
                        piece_kind,
                        point.x,
                        point.z,
                        height
                    );
                    height
                };
                snapped_points.push(Vector3::new(point.x, height, point.z));
            }
            let Some(snapped_loop) = Self::make_visual_polygon(snapped_points) else {
                return false;
            };
            snapped_loops.push(snapped_loop);
        }
        *loops = snapped_loops;
        true
    }

    pub(super) fn split_owned_region_contours_at_shared_vertices(
        owned_regions: &mut [NodeOwnedRegion],
    ) -> Option<()> {
        let split_keys = owned_regions
            .iter()
            .flat_map(|region| region.polygon.points_world.iter())
            .map(|point| Self::overlay_point_key([point.x, point.z]))
            .collect::<BTreeSet<_>>();
        Self::split_owned_region_contours_at_keys(owned_regions, &split_keys)
    }

    pub(super) fn insert_outer_boundary_vertices_into_owned_regions(
        owned_regions: &mut [NodeOwnedRegion],
    ) -> Option<()> {
        let visible_surface_polygons = owned_regions
            .iter()
            .map(|region| region.polygon.clone())
            .collect::<Vec<_>>();
        let outer_boundary_loops = Self::union_terrain_clip_polygons(&visible_surface_polygons);
        if outer_boundary_loops.is_empty() {
            return Some(());
        }

        let steiner_keys = outer_boundary_loops
            .iter()
            .flat_map(|polygon| polygon.points_world.iter())
            .map(|point| Self::overlay_point_key([point.x, point.z]))
            .collect::<BTreeSet<_>>();
        if steiner_keys.is_empty() {
            return Some(());
        }

        for region in owned_regions {
            let mut steiner_points = Vec::new();
            for key in &steiner_keys {
                let point_xz = Vector2::new(
                    key.0 as f32 / NODE_OVERLAY_SCALE,
                    key.1 as f32 / NODE_OVERLAY_SCALE,
                );
                if region
                    .polygon
                    .points_world
                    .iter()
                    .any(|point| Self::overlay_point_key([point.x, point.z]) == *key)
                {
                    continue;
                }
                if !Self::polygon_contour_contains_overlay_key(&region.polygon.points_world, *key)
                    && !Self::polygon_contains_point_xz(&region.polygon.points_world, point_xz)
                {
                    continue;
                }
                let Some(height) = Self::sample_height_from_candidate_coverage(
                    point_xz,
                    std::slice::from_ref(&region.polygon),
                ) else {
                    return None;
                };
                steiner_points.push(Vector3::new(point_xz.x, height, point_xz.y));
            }
            if steiner_points.is_empty() {
                continue;
            }
            Self::retriangulate_polygon_with_steiner_points(&mut region.polygon, &steiner_points)?;
        }
        Some(())
    }

    fn polygon_contour_contains_overlay_key(
        points_world: &[Vector3],
        key: NodeOverlayPointKey,
    ) -> bool {
        if points_world.len() < 2 {
            return false;
        }
        for index in 0..points_world.len() {
            let start = points_world[index];
            let end = points_world[(index + 1) % points_world.len()];
            let start_key = Self::overlay_point_key([start.x, start.z]);
            let end_key = Self::overlay_point_key([end.x, end.z]);
            if key == start_key
                || key == end_key
                || Self::overlay_key_lies_on_segment(key, start_key, end_key)
            {
                return true;
            }
        }
        false
    }

    fn retriangulate_polygon_with_steiner_points(
        polygon: &mut RoadSurfaceVisualPolygon,
        steiner_points: &[Vector3],
    ) -> Option<()> {
        if polygon.points_world.len() < 3 {
            return None;
        }
        let mut vertices = Vec::new();
        let mut vertex_lookup = BTreeMap::new();
        let mut constraints = BTreeSet::new();
        Self::push_surface_cdt_loop(
            &polygon.points_world,
            &mut vertices,
            &mut vertex_lookup,
            &mut constraints,
        );
        for &point in steiner_points {
            Self::insert_surface_cdt_vertex(point, &mut vertices, &mut vertex_lookup);
        }
        polygon.triangles_world = Self::triangulate_surface_cdt_vertices(
            vertices,
            constraints,
            &polygon.points_world,
            &[],
        )?;
        Some(())
    }

    fn split_owned_region_contours_at_keys(
        owned_regions: &mut [NodeOwnedRegion],
        split_keys: &BTreeSet<NodeOverlayPointKey>,
    ) -> Option<()> {
        if split_keys.is_empty() {
            return Some(());
        }
        for region in owned_regions {
            let points = &region.polygon.points_world;
            if points.len() < 3 {
                continue;
            }
            let mut split_points = Vec::new();
            for edge_index in 0..points.len() {
                let start = points[edge_index];
                let end = points[(edge_index + 1) % points.len()];
                let start_key = Self::overlay_point_key([start.x, start.z]);
                let end_key = Self::overlay_point_key([end.x, end.z]);
                if start_key == end_key {
                    continue;
                }

                let mut edge_samples = vec![(0.0_f32, start), (1.0_f32, end)];
                for &split_key in split_keys {
                    if split_key == start_key || split_key == end_key {
                        continue;
                    }
                    if !Self::overlay_key_lies_on_segment(split_key, start_key, end_key) {
                        continue;
                    }
                    let t = Self::overlay_key_segment_t(split_key, start_key, end_key);
                    if !(SAMPLE_EPSILON_M..=(1.0 - SAMPLE_EPSILON_M)).contains(&t) {
                        continue;
                    }
                    edge_samples.push((
                        t,
                        Vector3::new(
                            split_key.0 as f32 / NODE_OVERLAY_SCALE,
                            start.y + (end.y - start.y) * t,
                            split_key.1 as f32 / NODE_OVERLAY_SCALE,
                        ),
                    ));
                }
                edge_samples.sort_by(|a, b| {
                    a.0.total_cmp(&b.0)
                        .then(a.1.x.total_cmp(&b.1.x))
                        .then(a.1.z.total_cmp(&b.1.z))
                        .then(a.1.y.total_cmp(&b.1.y))
                });
                edge_samples.dedup_by(|a, b| {
                    Self::overlay_point_key([a.1.x, a.1.z])
                        == Self::overlay_point_key([b.1.x, b.1.z])
                });

                if split_points.is_empty() {
                    split_points.push(start);
                }
                split_points.extend(edge_samples.into_iter().skip(1).map(|(_, point)| point));
            }

            if let Some(split_polygon) = Self::make_visual_polygon(split_points) {
                region.polygon = split_polygon;
            } else {
                crate::debug_log!(
                    "road",
                    "node_owned_region_split_rejected kind={:?} owner_index={} original_vertices={}",
                    region.kind,
                    region.owner_index,
                    points.len()
                );
            }
        }
        Some(())
    }

    fn overlay_key_lies_on_segment(
        point: NodeOverlayPointKey,
        start: NodeOverlayPointKey,
        end: NodeOverlayPointKey,
    ) -> bool {
        let px = i128::from(point.0);
        let pz = i128::from(point.1);
        let ax = i128::from(start.0);
        let az = i128::from(start.1);
        let bx = i128::from(end.0);
        let bz = i128::from(end.1);
        let dx = bx - ax;
        let dz = bz - az;
        let cross = (px - ax) * dz - (pz - az) * dx;
        let collinear_tolerance = dx.abs().max(dz.abs()).max(1) * 4;
        if cross.abs() > collinear_tolerance {
            return false;
        }
        point.0 >= start.0.min(end.0)
            && point.0 <= start.0.max(end.0)
            && point.1 >= start.1.min(end.1)
            && point.1 <= start.1.max(end.1)
    }

    fn overlay_key_segment_t(
        point: NodeOverlayPointKey,
        start: NodeOverlayPointKey,
        end: NodeOverlayPointKey,
    ) -> f32 {
        let dx = (end.0 - start.0) as f32;
        let dz = (end.1 - start.1) as f32;
        if dx.abs() >= dz.abs() {
            if dx.abs() <= f32::EPSILON {
                0.0
            } else {
                (point.0 - start.0) as f32 / dx
            }
        } else if dz.abs() <= f32::EPSILON {
            0.0
        } else {
            (point.1 - start.1) as f32 / dz
        }
    }

    pub(super) fn normalize_owned_region_surface_heights(
        owned_regions: &mut [NodeOwnedRegion],
    ) -> bool {
        let mut height_by_key: BTreeMap<NodeOverlayPointKey, (f32, f32)> = BTreeMap::new();
        for point in owned_regions
            .iter()
            .map(|region| &region.polygon)
            .flat_map(|polygon| {
                polygon.points_world.iter().copied().chain(
                    polygon
                        .triangles_world
                        .iter()
                        .flat_map(|triangle| triangle.iter().copied()),
                )
            })
        {
            let key = Self::overlay_point_key([point.x, point.z]);
            height_by_key
                .entry(key)
                .and_modify(|(min_height, max_height)| {
                    *min_height = min_height.min(point.y);
                    *max_height = max_height.max(point.y);
                })
                .or_insert((point.y, point.y));
        }

        for polygon in owned_regions.iter_mut().map(|region| &mut region.polygon) {
            for point in &mut polygon.points_world {
                let key = Self::overlay_point_key([point.x, point.z]);
                if let Some((min_height, max_height)) = height_by_key.get(&key) {
                    if max_height - min_height <= NODE_SURFACE_HEIGHT_EPSILON_M {
                        point.y = *min_height;
                    }
                }
            }
            for triangle in &mut polygon.triangles_world {
                for point in triangle {
                    let key = Self::overlay_point_key([point.x, point.z]);
                    if let Some((min_height, max_height)) = height_by_key.get(&key) {
                        if max_height - min_height <= NODE_SURFACE_HEIGHT_EPSILON_M {
                            point.y = *min_height;
                        }
                    }
                }
            }
        }
        true
    }

    pub(super) fn weld_shared_top_surface_edges(owned_regions: &mut [NodeOwnedRegion]) {
        let mut edge_samples: BTreeMap<
            (NodeOverlayPointKey, NodeOverlayPointKey),
            BTreeMap<usize, ((u8, usize, usize), f32, f32)>,
        > = BTreeMap::new();

        for (region_index, region) in owned_regions.iter().enumerate() {
            let rank = (
                Self::band_kind_sort_key(region.kind),
                region.owner_index,
                region_index,
            );
            for triangle in &region.polygon.triangles_world {
                for edge_index in 0..3 {
                    let start = triangle[edge_index];
                    let end = triangle[(edge_index + 1) % 3];
                    let start_key = Self::overlay_point_key([start.x, start.z]);
                    let end_key = Self::overlay_point_key([end.x, end.z]);
                    if start_key == end_key {
                        continue;
                    }
                    let (key_a, key_b, height_a, height_b) = if start_key <= end_key {
                        (start_key, end_key, start.y, end.y)
                    } else {
                        (end_key, start_key, end.y, start.y)
                    };
                    edge_samples
                        .entry((key_a, key_b))
                        .or_default()
                        .entry(region_index)
                        .or_insert((rank, height_a, height_b));
                }
            }
        }

        let mut proposals: BTreeMap<NodeOverlayPointKey, Vec<((u8, usize, usize), f32)>> =
            BTreeMap::new();
        for ((key_a, key_b), samples_by_region) in edge_samples {
            if samples_by_region.len() < 2 {
                continue;
            }
            let mut min_a = f32::INFINITY;
            let mut max_a = f32::NEG_INFINITY;
            let mut min_b = f32::INFINITY;
            let mut max_b = f32::NEG_INFINITY;
            for (_, height_a, height_b) in samples_by_region.values() {
                min_a = min_a.min(*height_a);
                max_a = max_a.max(*height_a);
                min_b = min_b.min(*height_b);
                max_b = max_b.max(*height_b);
            }
            if max_a - min_a > NODE_SURFACE_HEIGHT_EPSILON_M
                || max_b - min_b > NODE_SURFACE_HEIGHT_EPSILON_M
            {
                continue;
            }
            let Some((rank, height_a, height_b)) = samples_by_region
                .values()
                .min_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then(a.1.total_cmp(&b.1))
                        .then(a.2.total_cmp(&b.2))
                })
                .copied()
            else {
                continue;
            };
            for (key, height) in [(key_a, height_a), (key_b, height_b)] {
                proposals.entry(key).or_default().push((rank, height));
            }
        }

        let targets = proposals
            .into_iter()
            .filter_map(|(key, mut values)| {
                let min_height = values
                    .iter()
                    .map(|(_, height)| *height)
                    .fold(f32::INFINITY, f32::min);
                let max_height = values
                    .iter()
                    .map(|(_, height)| *height)
                    .fold(f32::NEG_INFINITY, f32::max);
                if max_height - min_height > NODE_SURFACE_HEIGHT_EPSILON_M {
                    return None;
                }
                values.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
                values.first().copied().map(|target| (key, target))
            })
            .collect::<BTreeMap<_, _>>();

        if targets.is_empty() {
            return;
        }
        let target_heights = targets
            .iter()
            .map(|(key, (_, height))| (*key, *height))
            .collect::<BTreeMap<_, _>>();

        for region in owned_regions {
            let mut accepted_targets = target_heights
                .iter()
                .filter_map(|(key, height)| {
                    Self::region_accepts_weld_height(region, *key, *height, &target_heights)
                        .then_some((*key, *height))
                })
                .collect::<BTreeMap<_, _>>();
            loop {
                let before_len = accepted_targets.len();
                let snapshot = accepted_targets.clone();
                accepted_targets.retain(|key, height| {
                    Self::region_accepts_weld_height(region, *key, *height, &snapshot)
                });
                if accepted_targets.len() == before_len {
                    break;
                }
            }
            if accepted_targets.is_empty() {
                continue;
            }
            for point in &mut region.polygon.points_world {
                let key = Self::overlay_point_key([point.x, point.z]);
                if let Some(height) = accepted_targets.get(&key) {
                    point.y = *height;
                }
            }
            for triangle in &mut region.polygon.triangles_world {
                for point in triangle {
                    let key = Self::overlay_point_key([point.x, point.z]);
                    if let Some(height) = accepted_targets.get(&key) {
                        point.y = *height;
                    }
                }
            }
        }
    }

    fn region_accepts_weld_height(
        region: &NodeOwnedRegion,
        target_key: NodeOverlayPointKey,
        target_height: f32,
        targets: &BTreeMap<NodeOverlayPointKey, f32>,
    ) -> bool {
        let target_xz = Vector2::new(
            target_key.0 as f32 / NODE_OVERLAY_SCALE,
            target_key.1 as f32 / NODE_OVERLAY_SCALE,
        );
        for triangle in &region.polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                let start_key = Self::overlay_point_key([start.x, start.z]);
                let end_key = Self::overlay_point_key([end.x, end.z]);
                let (other, other_key) = if start_key == target_key && end_key != target_key {
                    (end, end_key)
                } else if end_key == target_key && start_key != target_key {
                    (start, start_key)
                } else {
                    continue;
                };
                let other_height = targets.get(&other_key).copied().unwrap_or(other.y);
                let xz_distance = Vector2::new(other.x, other.z).distance_to(target_xz);
                if xz_distance <= SAMPLE_EPSILON_M {
                    continue;
                }
                if (other_height - target_height).abs() / xz_distance
                    > NODE_SURFACE_SHARED_SEAM_WELD_MAX_SLOPE_RATIO
                {
                    return false;
                }
            }
        }
        true
    }

    fn overlay_contours_from_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for polygon in polygons {
            let contour = Self::overlay_contour_from_world_points(&polygon.points_world);
            if Self::overlay_contour_area(&contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                contours.push(contour);
            }
        }
        contours
    }

    fn overlay_contour_from_world_points(points_world: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points_world.len());
        for point in points_world {
            let overlay_point = Self::overlay_point_from_world_point(*point);
            if contour
                .last()
                .is_none_or(|last: &NodeOverlayPoint| *last != overlay_point)
            {
                contour.push(overlay_point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    fn overlay_point_from_world_point(point: Vector3) -> NodeOverlayPoint {
        [
            (point.x * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (point.z * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn overlay_union_contours(
        contours: &[NodeOverlayContour],
    ) -> Option<NodeOverlayShapes> {
        if contours.is_empty() {
            return Some(Vec::new());
        }
        let shapes = contours.simplify_shape(FillRule::Positive);
        Some(Self::filter_overlay_shapes_by_area(shapes))
    }

    fn overlay_binary_shapes(
        subject: &NodeOverlayShapes,
        clip: &NodeOverlayShapes,
        rule: OverlayRule,
    ) -> Option<NodeOverlayShapes> {
        if subject.is_empty() {
            return Some(Vec::new());
        }
        if clip.is_empty() {
            return Some(subject.clone());
        }
        let shapes = subject
            .overlay_with_fixed_scale(clip, rule, FillRule::Positive, NODE_OVERLAY_SCALE)
            .ok()?;
        Some(Self::filter_overlay_shapes_by_area(shapes))
    }

    fn filter_overlay_shapes_by_area(shapes: NodeOverlayShapes) -> NodeOverlayShapes {
        shapes
            .into_iter()
            .filter_map(|shape| {
                let filtered = shape
                    .into_iter()
                    .filter(|contour| contour.len() >= 3)
                    .collect::<Vec<_>>();
                let outer = filtered.first()?;
                (Self::overlay_contour_area(outer).abs() > NODE_OVERLAY_MIN_AREA_M2)
                    .then_some(filtered)
            })
            .collect()
    }

    fn sort_overlay_shapes(shapes: &mut [NodeOverlayShape]) {
        shapes.sort_by(|a, b| {
            let area_a = a
                .first()
                .map(|contour| Self::overlay_contour_area(contour).abs())
                .unwrap_or(0.0);
            let area_b = b
                .first()
                .map(|contour| Self::overlay_contour_area(contour).abs())
                .unwrap_or(0.0);
            area_b
                .total_cmp(&area_a)
                .then_with(|| Self::overlay_shape_sort_key(a).cmp(&Self::overlay_shape_sort_key(b)))
        });
    }

    fn overlay_shape_sort_key(shape: &NodeOverlayShape) -> (i64, i64, usize) {
        let mut min_x = i64::MAX;
        let mut min_z = i64::MAX;
        let mut points = 0usize;
        for contour in shape {
            points += contour.len();
            for point in contour {
                min_x = min_x.min((point[0] * NODE_OVERLAY_SCALE).round() as i64);
                min_z = min_z.min((point[1] * NODE_OVERLAY_SCALE).round() as i64);
            }
        }
        (min_x, min_z, points)
    }

    fn overlay_contour_area(contour: &NodeOverlayContour) -> f32 {
        if contour.len() < 3 {
            return 0.0;
        }
        let mut signed_area = 0.0;
        for index in 0..contour.len() {
            let current = contour[index];
            let next = contour[(index + 1) % contour.len()];
            signed_area += current[0] * next[1] - next[0] * current[1];
        }
        signed_area * 0.5
    }

    pub(super) fn union_terrain_clip_polygons(
        polygons: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        if polygons.is_empty() {
            return Vec::new();
        }

        let contours = Self::overlay_contours_from_polygons(polygons);
        let Some(mut shapes) = Self::overlay_union_contours(&contours) else {
            return Vec::new();
        };
        Self::sort_overlay_shapes(&mut shapes);
        Self::outer_boundary_polygons_from_overlay_shapes_with_candidate_heights(&shapes, polygons)
    }

    fn outer_boundary_polygons_from_overlay_shapes_with_candidate_heights(
        shapes: &[NodeOverlayShape],
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Vec<RoadSurfaceVisualPolygon> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) = Self::visual_polygon_from_overlay_shape_with_candidate_heights(
                shape, candidates, false,
            ) else {
                continue;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        polygons
    }

    #[cfg(test)]
    pub(super) fn visual_non_road_band_polygons_from_height_domains(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        target_non_road_shapes: &NodeOverlayShapes,
        road_shapes: &NodeOverlayShapes,
        height_domains: &[NodeGradeCarrier],
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        Some(
            Self::owned_non_road_regions_from_height_domains(
                node_id,
                piece_kind,
                target_non_road_shapes,
                road_shapes,
                height_domains,
            )?
            .into_iter()
            .map(|region| region.polygon)
            .collect(),
        )
    }

    fn owned_non_road_regions_from_height_domains(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        target_non_road_shapes: &NodeOverlayShapes,
        road_shapes: &NodeOverlayShapes,
        height_domains: &[NodeGradeCarrier],
    ) -> Option<Vec<NodeOwnedRegion>> {
        let mut owned_regions = Vec::new();
        let mut claimed_shapes = Vec::new();
        for kind in Self::non_road_visual_band_order() {
            let kind_domains = height_domains
                .iter()
                .enumerate()
                .filter(|(_, domain)| domain.kind == kind)
                .map(|(domain_index, domain)| (domain_index, domain.clone()))
                .collect::<Vec<_>>();
            if kind_domains.is_empty() {
                continue;
            }

            let contours = kind_domains
                .iter()
                .map(|(_, domain)| {
                    Self::overlay_contour_from_world_points(&domain.polygon.points_world)
                })
                .filter(|contour| {
                    Self::overlay_contour_area(contour).abs() > NODE_OVERLAY_MIN_AREA_M2
                })
                .collect::<Vec<_>>();
            let mut band_shapes = Self::overlay_union_contours(&contours)?;
            band_shapes = Self::overlay_binary_shapes(
                &band_shapes,
                target_non_road_shapes,
                OverlayRule::Intersect,
            )?;
            if !road_shapes.is_empty() {
                band_shapes = Self::overlay_binary_shapes(
                    &band_shapes,
                    road_shapes,
                    OverlayRule::Difference,
                )?;
            }
            if !claimed_shapes.is_empty() {
                band_shapes = Self::overlay_binary_shapes(
                    &band_shapes,
                    &claimed_shapes,
                    OverlayRule::Difference,
                )?;
            }
            Self::sort_overlay_shapes(&mut band_shapes);

            let mut kind_claimed_shapes = Vec::new();
            for (domain_index, domain) in &kind_domains {
                // Each domain owns its own source rails. Clip XZ ownership through overlay, then
                // sample the clipped fragment from that same domain instead of blending unrelated
                // elevated approach rails through the shared grade field.
                let contour = Self::overlay_contour_from_world_points(&domain.polygon.points_world);
                if Self::overlay_contour_area(&contour).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }

                let mut domain_shapes = Self::overlay_union_contours(&[contour])?;
                domain_shapes = Self::overlay_binary_shapes(
                    &domain_shapes,
                    &band_shapes,
                    OverlayRule::Intersect,
                )?;
                if !kind_claimed_shapes.is_empty() {
                    domain_shapes = Self::overlay_binary_shapes(
                        &domain_shapes,
                        &kind_claimed_shapes,
                        OverlayRule::Difference,
                    )?;
                }
                Self::sort_overlay_shapes(&mut domain_shapes);
                if domain_shapes.is_empty() {
                    continue;
                }

                let domain_polygons = Self::visual_polygons_from_overlay_shapes_with_height_domain(
                    node_id,
                    piece_kind,
                    "non_road_band",
                    kind,
                    &domain_shapes,
                    domain,
                )?;
                owned_regions.extend(domain_polygons.into_iter().map(|polygon| NodeOwnedRegion {
                    kind,
                    owner_index: *domain_index,
                    polygon,
                }));
                kind_claimed_shapes =
                    Self::overlay_union_shape_sets(&kind_claimed_shapes, &domain_shapes)?;
            }

            let kind_residual_shapes = if band_shapes.is_empty() {
                Vec::new()
            } else if kind_claimed_shapes.is_empty() {
                band_shapes.clone()
            } else {
                Self::overlay_binary_shapes(
                    &band_shapes,
                    &kind_claimed_shapes,
                    OverlayRule::Difference,
                )?
            };
            if !kind_residual_shapes.is_empty() {
                let residual_area_m2 = kind_residual_shapes
                    .iter()
                    .map(Self::overlay_shape_area_m2)
                    .sum::<f32>();
                crate::debug_log!(
                    "road",
                    "node_non_road_unowned_kind_residual node={} piece={:?} kind={:?} shape_count={} area_m2={:.3}",
                    node_id,
                    piece_kind,
                    kind,
                    kind_residual_shapes.len(),
                    residual_area_m2
                );
                return None;
            }

            claimed_shapes = Self::overlay_union_shape_sets(&claimed_shapes, &band_shapes)?;
        }

        let residual_shapes = if target_non_road_shapes.is_empty() {
            Vec::new()
        } else if claimed_shapes.is_empty() {
            target_non_road_shapes.clone()
        } else {
            Self::overlay_binary_shapes(
                target_non_road_shapes,
                &claimed_shapes,
                OverlayRule::Difference,
            )?
        };
        if !residual_shapes.is_empty() {
            let residual_area_m2 = residual_shapes
                .iter()
                .map(Self::overlay_shape_area_m2)
                .sum::<f32>();
            crate::debug_log!(
                "road",
                "node_non_road_unowned_residual node={} piece={:?} shape_count={} area_m2={:.3}",
                node_id,
                piece_kind,
                residual_shapes.len(),
                residual_area_m2
            );
            return None;
        }
        Self::sort_node_owned_regions(&mut owned_regions);
        Some(owned_regions)
    }

    fn owned_regions_from_height_domains(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        material_name: &'static str,
        target_shapes: &NodeOverlayShapes,
        height_domains: &[NodeGradeCarrier],
    ) -> Option<Vec<NodeOwnedRegion>> {
        let mut owned_regions = Vec::new();
        let mut claimed_shapes = Vec::new();
        for (domain_index, domain) in height_domains.iter().enumerate() {
            let contour = Self::overlay_contour_from_world_points(&domain.polygon.points_world);
            if Self::overlay_contour_area(&contour).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                continue;
            }

            let mut domain_shapes = Self::overlay_union_contours(&[contour])?;
            domain_shapes =
                Self::overlay_binary_shapes(&domain_shapes, target_shapes, OverlayRule::Intersect)?;
            if !claimed_shapes.is_empty() {
                domain_shapes = Self::overlay_binary_shapes(
                    &domain_shapes,
                    &claimed_shapes,
                    OverlayRule::Difference,
                )?;
            }
            Self::sort_overlay_shapes(&mut domain_shapes);
            if domain_shapes.is_empty() {
                continue;
            }

            let domain_polygons = Self::visual_polygons_from_overlay_shapes_with_height_domain(
                node_id,
                piece_kind,
                material_name,
                domain.kind,
                &domain_shapes,
                domain,
            )?;
            owned_regions.extend(domain_polygons.into_iter().map(|polygon| NodeOwnedRegion {
                kind: domain.kind,
                owner_index: domain_index,
                polygon,
            }));
            claimed_shapes = Self::overlay_union_shape_sets(&claimed_shapes, &domain_shapes)?;
        }

        let residual_shapes = if target_shapes.is_empty() {
            Vec::new()
        } else if claimed_shapes.is_empty() {
            target_shapes.clone()
        } else {
            Self::overlay_binary_shapes(target_shapes, &claimed_shapes, OverlayRule::Difference)?
        };
        if !residual_shapes.is_empty() {
            let residual_area_m2 = residual_shapes
                .iter()
                .map(Self::overlay_shape_area_m2)
                .sum::<f32>();
            crate::debug_log!(
                "road",
                "node_owned_domain_residual node={} piece={:?} material={} shape_count={} area_m2={:.3}",
                node_id,
                piece_kind,
                material_name,
                residual_shapes.len(),
                residual_area_m2
            );
            return None;
        }
        Self::sort_node_owned_regions(&mut owned_regions);
        Some(owned_regions)
    }

    fn visual_polygons_from_overlay_shapes_with_height_domain(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        material_name: &'static str,
        material_kind: RoadSurfaceBandKind,
        shapes: &[NodeOverlayShape],
        domain: &NodeGradeCarrier,
    ) -> Option<Vec<RoadSurfaceVisualPolygon>> {
        let mut polygons = Vec::new();
        for shape in shapes {
            let Some(polygon) = Self::visual_polygon_from_overlay_shape_with_candidate_heights(
                shape,
                std::slice::from_ref(&domain.polygon),
                true,
            ) else {
                crate::debug_log!(
                    "road",
                    "node_domain_height_missing node={} piece={:?} material={} kind={:?}",
                    node_id,
                    piece_kind,
                    material_name,
                    material_kind
                );
                return None;
            };
            polygons.push(polygon);
        }
        Self::sort_visual_polygons(&mut polygons);
        Some(polygons)
    }

    fn overlay_union_shape_sets(
        existing: &NodeOverlayShapes,
        added: &NodeOverlayShapes,
    ) -> Option<NodeOverlayShapes> {
        if existing.is_empty() {
            return Some(added.clone());
        }
        if added.is_empty() {
            return Some(existing.clone());
        }
        let contours = existing
            .iter()
            .chain(added.iter())
            .flat_map(|shape| shape.iter().cloned())
            .collect::<Vec<_>>();
        Self::overlay_union_contours(&contours)
    }

    fn overlay_shape_area_m2(shape: &NodeOverlayShape) -> f32 {
        let Some(outer) = shape.first() else {
            return 0.0;
        };
        let holes = shape
            .iter()
            .skip(1)
            .map(|hole| Self::overlay_contour_area(hole).abs())
            .sum::<f32>();
        (Self::overlay_contour_area(outer).abs() - holes).max(0.0)
    }

    fn overlay_point_key(point: NodeOverlayPoint) -> NodeOverlayPointKey {
        (
            (point[0] * NODE_OVERLAY_SCALE).round() as i64,
            (point[1] * NODE_OVERLAY_SCALE).round() as i64,
        )
    }

    fn non_road_visual_band_order() -> [RoadSurfaceBandKind; 7] {
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

    pub(super) fn band_kind_sort_key(kind: RoadSurfaceBandKind) -> u8 {
        match kind {
            RoadSurfaceBandKind::Carriageway => 0,
            RoadSurfaceBandKind::CurbOrShoulder => 1,
            RoadSurfaceBandKind::Sidewalk => 2,
            RoadSurfaceBandKind::Footpath => 3,
            RoadSurfaceBandKind::Median => 4,
            RoadSurfaceBandKind::Parking => 5,
            RoadSurfaceBandKind::CycleTrack => 6,
            RoadSurfaceBandKind::TramReservation => 7,
        }
    }

    fn visual_polygon_from_overlay_shape_with_candidate_heights(
        shape: &NodeOverlayShape,
        candidates: &[RoadSurfaceVisualPolygon],
        preserve_holes: bool,
    ) -> Option<RoadSurfaceVisualPolygon> {
        let outer_contour = shape.first()?;
        let mut outer_points = Self::world_points_from_overlay_contour_with_candidate_heights(
            outer_contour,
            candidates,
        )?;
        if Self::signed_polygon_area_xz(&outer_points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
            return None;
        }
        if Self::signed_polygon_area_xz(&outer_points) < 0.0 {
            outer_points.reverse();
        }

        let mut hole_points = Vec::new();
        if preserve_holes {
            for contour in shape.iter().skip(1) {
                let mut points = Self::world_points_from_overlay_contour_with_candidate_heights(
                    contour, candidates,
                )?;
                if Self::signed_polygon_area_xz(&points).abs() <= NODE_OVERLAY_MIN_AREA_M2 {
                    continue;
                }
                if Self::signed_polygon_area_xz(&points) > 0.0 {
                    points.reverse();
                }
                hole_points.push(points);
            }
        }

        Self::canonicalize_world_loop(&mut outer_points)?;
        for hole in &mut hole_points {
            Self::canonicalize_world_loop(hole)?;
        }
        let triangles_world = Self::triangulate_constrained_shape_xz(&outer_points, &hole_points)?;
        Some(RoadSurfaceVisualPolygon {
            points_world: outer_points,
            triangles_world,
        })
    }

    fn world_points_from_overlay_contour_with_candidate_heights(
        contour: &NodeOverlayContour,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<Vec<Vector3>> {
        contour
            .iter()
            .map(|point| {
                let xz = Vector2::new(point[0], point[1]);
                let y = Self::sample_height_from_candidate_coverage(xz, candidates)?;
                Some(Vector3::new(point[0], y, point[1]))
            })
            .collect()
    }

    fn canonicalize_world_loop(points_world: &mut Vec<Vector3>) -> Option<()> {
        points_world
            .dedup_by(|a, b| (*a - *b).length_squared() <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2);
        if points_world.len() >= 2
            && (points_world.first().copied()? - points_world.last().copied()?).length_squared()
                <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
        {
            points_world.pop();
        }
        if points_world.len() < 3 {
            return None;
        }
        let (start_index, _) = points_world.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.x.total_cmp(&b.x)
                .then(a.z.total_cmp(&b.z))
                .then(a.y.total_cmp(&b.y))
        })?;
        points_world.rotate_left(start_index);
        Some(())
    }

    fn sample_height_from_candidate_coverage(
        point_xz: Vector2,
        candidates: &[RoadSurfaceVisualPolygon],
    ) -> Option<f32> {
        let point_key = Self::overlay_point_key([point_xz.x, point_xz.y]);
        let mut vertex_heights = Vec::new();
        for polygon in candidates {
            for point in &polygon.points_world {
                if Self::overlay_point_key([point.x, point.z]) == point_key {
                    vertex_heights.push(point.y);
                }
            }
            for triangle in &polygon.triangles_world {
                for point in triangle {
                    if Self::overlay_point_key([point.x, point.z]) == point_key {
                        vertex_heights.push(point.y);
                    }
                }
            }
        }
        if !vertex_heights.is_empty() {
            return Self::canonical_height_sample(vertex_heights);
        }

        let mut covered_heights = Vec::new();
        for polygon in candidates {
            for triangle in &polygon.triangles_world {
                if let Some((wa, wb, wc)) =
                    Self::triangle_barycentric_weights_xz(*triangle, point_xz)
                {
                    covered_heights
                        .push(triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc);
                }
            }
        }
        Self::canonical_height_sample(covered_heights)
    }

    fn canonical_height_sample<I>(heights: I) -> Option<f32>
    where
        I: IntoIterator<Item = f32>,
    {
        let mut heights = heights.into_iter().collect::<Vec<_>>();
        if heights.is_empty() {
            return None;
        }
        heights.sort_by(|a, b| a.total_cmp(b));
        let min_height = *heights.first()?;
        let max_height = *heights.last()?;
        (max_height - min_height <= NODE_SURFACE_HEIGHT_EPSILON_M).then_some(min_height)
    }

    fn canonical_height_sample_for_reference<I>(heights: I, reference_height: f32) -> Option<f32>
    where
        I: IntoIterator<Item = f32>,
    {
        let mut matching_heights = heights
            .into_iter()
            .filter(|height| (*height - reference_height).abs() <= NODE_SURFACE_HEIGHT_EPSILON_M)
            .collect::<Vec<_>>();
        if matching_heights.is_empty() {
            return None;
        }
        matching_heights.sort_by(|a, b| a.total_cmp(b));
        let min_height = *matching_heights.first()?;
        let max_height = *matching_heights.last()?;
        (max_height - min_height <= NODE_SURFACE_HEIGHT_EPSILON_M).then_some(min_height)
    }

    fn triangulate_constrained_shape_xz(
        outer_points: &[Vector3],
        holes: &[Vec<Vector3>],
    ) -> Option<Vec<[Vector3; 3]>> {
        if outer_points.len() < 3 {
            return None;
        }

        let mut vertices = Vec::new();
        let mut vertex_lookup = BTreeMap::new();
        let mut constraints = BTreeSet::new();
        Self::push_surface_cdt_loop(
            outer_points,
            &mut vertices,
            &mut vertex_lookup,
            &mut constraints,
        );
        for hole in holes {
            Self::push_surface_cdt_loop(hole, &mut vertices, &mut vertex_lookup, &mut constraints);
        }

        Self::triangulate_surface_cdt_vertices(vertices, constraints, outer_points, holes)
    }

    fn triangulate_surface_cdt_vertices(
        vertices: Vec<Vector3>,
        constraints: BTreeSet<[usize; 2]>,
        outer_points: &[Vector3],
        holes: &[Vec<Vector3>],
    ) -> Option<Vec<[Vector3; 3]>> {
        let spade_vertices = vertices
            .iter()
            .map(|point| Point2::new(f64::from(point.x), f64::from(point.z)))
            .collect::<Vec<_>>();
        let mut invalid_constraints = 0usize;
        let cdt = SurfaceCdt::try_bulk_load_cdt(
            spade_vertices,
            constraints.into_iter().collect(),
            |_| invalid_constraints += 1,
        )
        .ok()?;
        if invalid_constraints > 0 {
            return None;
        }

        let mut triangles = Vec::new();
        for face in cdt.inner_faces() {
            let [a, b, c] = face.vertices();
            let triangle = [
                vertices[a.fix().index()],
                vertices[b.fix().index()],
                vertices[c.fix().index()],
            ];
            let centroid = Vector2::new(
                (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            );
            if !Self::triangle_has_area_xz(triangle) {
                continue;
            }
            if !Self::polygon_contains_point_xz(outer_points, centroid) {
                continue;
            }
            if holes
                .iter()
                .any(|hole| Self::polygon_contains_point_xz(hole, centroid))
            {
                continue;
            }
            triangles.push(triangle);
        }

        (!triangles.is_empty()).then_some(triangles)
    }

    fn push_surface_cdt_loop(
        points_world: &[Vector3],
        vertices: &mut Vec<Vector3>,
        vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
        constraints: &mut BTreeSet<[usize; 2]>,
    ) {
        if points_world.len() < 2 {
            return;
        }
        let indices = points_world
            .iter()
            .map(|point| Self::insert_surface_cdt_vertex(*point, vertices, vertex_lookup))
            .collect::<Vec<_>>();
        for index in 0..indices.len() {
            let edge = Self::normalize_surface_edge_array(
                indices[index],
                indices[(index + 1) % indices.len()],
            );
            if edge[0] != edge[1] {
                constraints.insert(edge);
            }
        }
    }

    fn insert_surface_cdt_vertex(
        point: Vector3,
        vertices: &mut Vec<Vector3>,
        vertex_lookup: &mut BTreeMap<(i64, i64), usize>,
    ) -> usize {
        let key = Self::surface_cdt_vertex_key(point);
        if let Some(index) = vertex_lookup.get(&key) {
            return *index;
        }
        let index = vertices.len();
        vertices.push(point);
        vertex_lookup.insert(key, index);
        index
    }

    fn surface_cdt_vertex_key(point: Vector3) -> (i64, i64) {
        (
            (point.x / SAMPLE_EPSILON_M).round() as i64,
            (point.z / SAMPLE_EPSILON_M).round() as i64,
        )
    }

    fn normalize_surface_edge_array(a: usize, b: usize) -> [usize; 2] {
        if a < b { [a, b] } else { [b, a] }
    }
}
