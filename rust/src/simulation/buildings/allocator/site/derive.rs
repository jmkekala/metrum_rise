//! Derivation of deterministic flat support footprints from building assets.

use super::geometry::{
    SITE_POINT_EPS_SQUARED_M2, convex_hull_from_sorted_points, signed_polygon_area, site_radius_m,
};
use super::model::{BuildingSiteClient, BuildingSiteSurfaceClient};
use crate::assets::{Anchor, AnchorType, AssetManifest, MeshPart, SiteSurface};
use crate::simulation::buildings::allocator::entrance::{
    building_local_xz_basis, building_local_xz_pos,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use godot::prelude::Vector2;

const DEFAULT_ANCHOR_FORWARD: [f32; 3] = [0.0, 0.0, 1.0];
const BUILDING_SITE_MESH_PART_SUPPORT_MARGIN_M: f32 = 5.25;
const BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M: f32 = 0.35;
const BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M: f32 = 0.25;
pub(super) const BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M: f32 = 1.0;
const BUILDING_SITE_ENTRANCE_SUPPORT_WIDTH_M: f32 = 2.0;
const BUILDING_SITE_ENTRANCE_SUPPORT_LENGTH_M: f32 = 2.0;
const BUILDING_SITE_DEFAULT_SUPPORT_INSET_M: f32 = 2.0;

#[derive(Clone, Copy)]
struct LocalLotSupport {
    half_width: f32,
    half_depth: f32,
    frontage_dir: Vector2,
}

impl LocalLotSupport {
    fn clamp_point(self, point: Vector2) -> Vector2 {
        Vector2::new(
            point.x.clamp(-self.half_width, self.half_width),
            point.y.clamp(-self.half_depth, self.half_depth),
        )
    }

    fn clamp_road_access_point(self, point: Vector2) -> Vector2 {
        let point = self.clamp_point(point);
        if self.frontage_dir.length_squared() <= f32::EPSILON {
            return point;
        }
        let frontage_dir = self.frontage_dir.normalized();
        let front_limit = frontage_projection_limit(frontage_dir, self.half_width, self.half_depth);
        let clearance = BUILDING_SITE_ROAD_ACCESS_CLEARANCE_M.min(front_limit.max(0.0) * 0.5);
        let support_limit = front_limit - clearance;
        let projection = frontage_projection(point, frontage_dir);
        if projection <= support_limit {
            return point;
        }
        self.clamp_point(point - frontage_dir * (projection - support_limit))
    }
}

impl BuildingAllocator {
    pub(crate) fn rebuild_building_site_clients(&mut self, zone_cell_m: f32) {
        self.building_sites = self
            .buildings
            .iter()
            .map(|building| self.derive_building_site_client(building, zone_cell_m))
            .collect();
        self.recompute_max_site_radius_m();
    }

    pub(crate) fn rebuild_building_site_client(&mut self, building_idx: usize, zone_cell_m: f32) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let client = self.derive_building_site_client(&self.buildings[building_idx], zone_cell_m);
        if self.building_sites.len() != self.buildings.len() {
            self.rebuild_building_site_clients(zone_cell_m);
        } else if let Some(slot) = self.building_sites.get_mut(building_idx) {
            *slot = client;
            self.recompute_max_site_radius_m();
        }
    }

    pub(crate) fn push_building_site_client(&mut self, building_idx: usize, zone_cell_m: f32) {
        let client = self.derive_building_site_client(&self.buildings[building_idx], zone_cell_m);
        if self.building_sites.len() == building_idx {
            self.max_site_radius_m = self.max_site_radius_m.max(site_radius_m(&client));
            self.building_sites.push(client);
        } else {
            self.rebuild_building_site_clients(zone_cell_m);
        }
    }

    pub(crate) fn recompute_max_site_radius_m(&mut self) {
        self.max_site_radius_m = self
            .building_sites
            .iter()
            .map(site_radius_m)
            .fold(0.0, f32::max);
    }

    pub(super) fn derive_building_site_client(
        &self,
        building: &Building,
        zone_cell_m: f32,
    ) -> BuildingSiteClient {
        let frontage_forward = self
            .registry
            .get(&building.asset_id)
            .map(|entry| entry.manifest.building_frontage_forward())
            .unwrap_or(DEFAULT_ANCHOR_FORWARD);
        let (basis_x, basis_z) = building_local_xz_basis(building.facing_dir, frontage_forward);
        let center = Vector2::new(building.center_x, building.center_y);
        let lot_half_width = building.width_cells as f32 * zone_cell_m * 0.5;
        let lot_half_depth = building.depth_cells as f32 * zone_cell_m * 0.5;
        let lot_footprint_world = [
            center + basis_x * -lot_half_width + basis_z * -lot_half_depth,
            center + basis_x * -lot_half_width + basis_z * lot_half_depth,
            center + basis_x * lot_half_width + basis_z * lot_half_depth,
            center + basis_x * lot_half_width + basis_z * -lot_half_depth,
        ];
        let footprint_world = self.required_flat_support_footprint_world(
            &building.asset_id,
            center,
            building.facing_dir,
            building.width_cells as usize,
            building.depth_cells as usize,
            zone_cell_m,
        );
        let surfaces = self
            .registry
            .get(&building.asset_id)
            .map(|entry| {
                entry
                    .manifest
                    .site_surfaces
                    .iter()
                    .map(|surface| BuildingSiteSurfaceClient {
                        material: surface.material,
                        name: surface.name.clone(),
                        height_m: building.support_height_m + surface.y_m,
                        vertices_world: surface
                            .vertices
                            .iter()
                            .map(|vertex| {
                                building_local_xz_pos(
                                    building,
                                    [vertex[0], 0.0, vertex[1]],
                                    frontage_forward,
                                )
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        BuildingSiteClient {
            footprint_world,
            lot_footprint_world,
            support_height_m: building.support_height_m,
            surfaces,
        }
    }

    pub(crate) fn required_flat_support_footprint_world(
        &self,
        asset_id: &str,
        center: Vector2,
        facing_dir: Vector2,
        width_cells: usize,
        depth_cells: usize,
        zone_cell_m: f32,
    ) -> Vec<Vector2> {
        let lot_half_width = width_cells as f32 * zone_cell_m * 0.5;
        let lot_half_depth = depth_cells as f32 * zone_cell_m * 0.5;
        let frontage_forward = self
            .registry
            .get(asset_id)
            .map(|entry| entry.manifest.building_frontage_forward())
            .unwrap_or(DEFAULT_ANCHOR_FORWARD);
        let (basis_x, basis_z) = building_local_xz_basis(facing_dir, frontage_forward);
        let footprint_local = self
            .registry
            .get(asset_id)
            .map(|entry| {
                required_flat_support_footprint_local(
                    &entry.manifest,
                    lot_half_width,
                    lot_half_depth,
                )
            })
            .unwrap_or_else(|| lot_footprint_local(lot_half_width, lot_half_depth));
        footprint_local
            .into_iter()
            .map(|point| center + basis_x * point.x + basis_z * point.y)
            .collect()
    }
}

pub(super) fn required_flat_support_footprint_local(
    manifest: &AssetManifest,
    lot_half_width: f32,
    lot_half_depth: f32,
) -> Vec<Vector2> {
    let lot = LocalLotSupport {
        half_width: lot_half_width,
        half_depth: lot_half_depth,
        frontage_dir: asset_frontage_dir_local(manifest),
    };
    let mut points = Vec::new();
    for part in &manifest.mesh_parts {
        append_mesh_part_support_points(part, lot, &mut points);
    }
    for anchor in &manifest.anchors {
        append_anchor_support_points(anchor, lot, &mut points);
    }
    for surface in &manifest.site_surfaces {
        append_site_surface_support_points(surface, lot, &mut points);
    }
    if points.is_empty() {
        return default_support_footprint_local(lot_half_width, lot_half_depth);
    }

    points.sort_by(|left, right| {
        left.x
            .total_cmp(&right.x)
            .then_with(|| left.y.total_cmp(&right.y))
    });
    points.dedup_by(|left, right| left.distance_squared_to(*right) <= SITE_POINT_EPS_SQUARED_M2);
    let mut hull = convex_hull_from_sorted_points(points);
    if hull.len() < 3 || signed_polygon_area(&hull).abs() <= SITE_POINT_EPS_SQUARED_M2 {
        hull = default_support_footprint_local(lot_half_width, lot_half_depth);
    }
    if signed_polygon_area(&hull) < 0.0 {
        hull.reverse();
    }
    hull
}

fn asset_frontage_dir_local(manifest: &AssetManifest) -> Vector2 {
    let front = manifest.building_frontage_forward();
    let front = Vector2::new(front[0], front[2]);
    if front.length_squared() > f32::EPSILON {
        front.normalized()
    } else {
        Vector2::new(DEFAULT_ANCHOR_FORWARD[0], DEFAULT_ANCHOR_FORWARD[2])
    }
}

fn append_mesh_part_support_points(
    part: &MeshPart,
    lot: LocalLotSupport,
    points: &mut Vec<Vector2>,
) {
    let half_extent = (part.scale * 0.75).max(BUILDING_SITE_MESH_PART_SUPPORT_MARGIN_M);
    append_axis_aligned_support_rect(
        Vector2::new(part.position[0], part.position[2]),
        half_extent,
        half_extent,
        lot,
        points,
    );
}

fn append_anchor_support_points(anchor: &Anchor, lot: LocalLotSupport, points: &mut Vec<Vector2>) {
    match anchor.anchor_type {
        AnchorType::Entrance => append_oriented_support_rect(
            anchor.position,
            anchor.forward,
            anchor
                .width_m
                .unwrap_or(BUILDING_SITE_ENTRANCE_SUPPORT_WIDTH_M),
            BUILDING_SITE_ENTRANCE_SUPPORT_LENGTH_M,
            BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M,
            lot,
            points,
        ),
        AnchorType::Driveway => {
            let width = anchor.width_m.unwrap_or(0.0);
            append_oriented_support_rect(
                anchor.position,
                anchor.forward,
                width,
                (width * 1.4).max(1.5),
                BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M,
                lot,
                points,
            );
        }
        AnchorType::Parking | AnchorType::LoadingBay => append_oriented_support_rect(
            anchor.position,
            anchor.forward,
            anchor.width_m.unwrap_or(0.0),
            anchor.length_m.unwrap_or(0.0),
            BUILDING_SITE_ACCESS_SUPPORT_MARGIN_M,
            lot,
            points,
        ),
        AnchorType::Wheel | AnchorType::Light => {}
    }
}

fn append_site_surface_support_points(
    surface: &SiteSurface,
    lot: LocalLotSupport,
    points: &mut Vec<Vector2>,
) {
    if surface.vertices.is_empty() {
        return;
    }
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for vertex in &surface.vertices {
        min_x = min_x.min(vertex[0]);
        min_z = min_z.min(vertex[1]);
        max_x = max_x.max(vertex[0]);
        max_z = max_z.max(vertex[1]);
    }
    min_x -= BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    min_z -= BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    max_x += BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    max_z += BUILDING_SITE_SURFACE_SUPPORT_MARGIN_M;
    for point in [
        Vector2::new(min_x, min_z),
        Vector2::new(min_x, max_z),
        Vector2::new(max_x, max_z),
        Vector2::new(max_x, min_z),
    ] {
        points.push(lot.clamp_point(point));
    }
}

fn append_axis_aligned_support_rect(
    center: Vector2,
    half_width: f32,
    half_depth: f32,
    lot: LocalLotSupport,
    points: &mut Vec<Vector2>,
) {
    for point in [
        Vector2::new(center.x - half_width, center.y - half_depth),
        Vector2::new(center.x - half_width, center.y + half_depth),
        Vector2::new(center.x + half_width, center.y + half_depth),
        Vector2::new(center.x + half_width, center.y - half_depth),
    ] {
        points.push(lot.clamp_point(point));
    }
}

fn append_oriented_support_rect(
    position: [f32; 3],
    forward: [f32; 3],
    width_m: f32,
    length_m: f32,
    margin_m: f32,
    lot: LocalLotSupport,
    points: &mut Vec<Vector2>,
) {
    let forward = Vector2::new(forward[0], forward[2]);
    if forward.length_squared() <= f32::EPSILON {
        return;
    }
    let forward = forward.normalized();
    let side = Vector2::new(-forward.y, forward.x);
    let origin = Vector2::new(position[0], position[2]);
    let half_width = width_m.max(0.0) * 0.5 + margin_m.max(0.0);
    let back = margin_m.max(0.0);
    let front = length_m.max(0.0) + margin_m.max(0.0);
    for point in [
        origin - side * half_width - forward * back,
        origin + side * half_width - forward * back,
        origin + side * half_width + forward * front,
        origin - side * half_width + forward * front,
    ] {
        points.push(lot.clamp_road_access_point(point));
    }
}

pub(super) fn frontage_projection(point: Vector2, frontage_dir: Vector2) -> f32 {
    point.x * frontage_dir.x + point.y * frontage_dir.y
}

pub(super) fn frontage_projection_limit(
    frontage_dir: Vector2,
    lot_half_width: f32,
    lot_half_depth: f32,
) -> f32 {
    frontage_dir.x.abs() * lot_half_width + frontage_dir.y.abs() * lot_half_depth
}

fn default_support_footprint_local(lot_half_width: f32, lot_half_depth: f32) -> Vec<Vector2> {
    let inset = BUILDING_SITE_DEFAULT_SUPPORT_INSET_M
        .min(lot_half_width.max(0.0) * 0.5)
        .min(lot_half_depth.max(0.0) * 0.5);
    lot_footprint_local(lot_half_width - inset, lot_half_depth - inset)
}

fn lot_footprint_local(lot_half_width: f32, lot_half_depth: f32) -> Vec<Vector2> {
    vec![
        Vector2::new(-lot_half_width, -lot_half_depth),
        Vector2::new(-lot_half_width, lot_half_depth),
        Vector2::new(lot_half_width, lot_half_depth),
        Vector2::new(lot_half_width, -lot_half_depth),
    ]
}
