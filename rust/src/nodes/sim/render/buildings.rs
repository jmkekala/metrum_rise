// SPDX-License-Identifier: GPL-2.0-only

//! Building-specific rendering logic for Godot interaction.
//!
//! Handles building instance transform generation and plot/construction-site visuals.

use crate::assets::{AssetEntry, MeshPart, SiteSurfaceMaterial};
use crate::config::{HEIGHT_SCALE, SIDEWALK_WIDTH};
use crate::nodes::sim::core::SimCore;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, building_local_xz_basis,
};
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use godot::prelude::*;

const SITE_INSET_SCALE: f32 = 0.92;
const FOUNDATION_INSET_SCALE: f32 = 0.68;
const SITE_PAD_HEIGHT_M: f32 = 0.08;
const FOUNDATION_PAD_HEIGHT_M: f32 = 0.18;
const SCAFFOLD_POST_THICKNESS_M: f32 = 0.28;
const SCAFFOLD_RAIL_THICKNESS_M: f32 = 0.22;
const BUILDING_SITE_DEBUG_EDGE_SAMPLE_STEP_M: f32 = 5.0;
const BUILDING_SITE_DEBUG_EDGE_SAMPLE_MAX: usize = 9;
const BUILDING_SITE_DEBUG_ROAD_SAMPLE_INSET_M: f32 = 0.05;
const BUILDING_SITE_DEBUG_NEAREST_ROAD_SURFACE_RADIUS_M: f32 = 8.0;
const CONSTRUCTION_RISE_INITIAL_HEIGHT_FACTOR: f32 = 0.65;
const CONSTRUCTION_RISE_INITIAL_EXTRA_M: f32 = 2.0;
const CONSTRUCTION_RISE_INITIAL_MIN_OFFSET_M: f32 = 6.0;
const CONSTRUCTION_RISE_INITIAL_MAX_OFFSET_M: f32 = 12.0;

impl SimCore {
    // ── Building Renderer ──

    /// Returns the 12-float transforms for all placed buildings with the given asset part.
    pub fn get_building_transforms_for_asset_part_internal(
        &self,
        asset_id: &str,
        part_index: i32,
    ) -> PackedFloat32Array {
        if part_index < 0 {
            return PackedFloat32Array::new();
        }
        let mut buffer = Vec::new();
        let entry = self.allocator.registry.get(asset_id);
        let part = entry.and_then(|entry| entry.manifest.mesh_parts.get(part_index as usize));

        for b in &self.allocator.buildings {
            if asset_id == "broken:error" {
                if part_index != 0 || !b.broken || b.is_under_construction() {
                    continue;
                }
            } else {
                // Skip broken buildings (handled by broken:error group) and deserted buildings
                // (handled by the parallel deserted multimesh via get_deserted_building_transforms_for_asset_internal).
                if b.broken || b.is_deserted || b.asset_id != asset_id {
                    continue;
                }
            }

            let mut world_y = b.support_height_m;
            if b.is_under_construction() {
                let progress = construction_visual_progress(b, self.operational_hour_fraction());
                world_y -= construction_rise_offset_m(b, progress);
            }
            if let Some(part) = part {
                push_building_part_transform(&mut buffer, b, entry, part, world_y);
            } else if asset_id == "broken:error" {
                push_broken_building_transform(&mut buffer, b, world_y);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the 12-float transforms for all deserted buildings with the given asset part.
    ///
    /// Deserted buildings render in a parallel multimesh with a gray material override.
    pub fn get_deserted_building_transforms_for_asset_part_internal(
        &self,
        asset_id: &str,
        part_index: i32,
    ) -> PackedFloat32Array {
        if part_index < 0 {
            return PackedFloat32Array::new();
        }
        let mut buffer = Vec::new();
        let entry = self.allocator.registry.get(asset_id);
        let part = entry.and_then(|entry| entry.manifest.mesh_parts.get(part_index as usize));
        let Some(part) = part else {
            return PackedFloat32Array::new();
        };

        for b in &self.allocator.buildings {
            if b.broken || b.is_under_construction() || !b.is_deserted || b.asset_id != asset_id {
                continue;
            }

            let world_y = b.support_height_m;
            push_building_part_transform(&mut buffer, b, entry, part, world_y);
        }

        PackedFloat32Array::from_iter(buffer)
    }

    pub(crate) fn get_service_building_preview_part_transforms_internal(
        &self,
        asset_id: &str,
        center_2d: Vector2,
        support_height_m: f32,
        facing_dir: Vector2,
    ) -> PackedFloat32Array {
        let Some(entry) = self.allocator.registry.get(asset_id) else {
            return PackedFloat32Array::new();
        };

        let mut buffer = Vec::with_capacity(entry.manifest.mesh_parts.len() * 12);
        for part in &entry.manifest.mesh_parts {
            push_building_part_transform_for_pose(
                &mut buffer,
                center_2d,
                support_height_m,
                facing_dir,
                Some(entry),
                part,
            );
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns the revision used by Godot to decide whether site meshes need rebuilding.
    pub fn get_building_site_revision_internal(&self) -> u64 {
        self.allocator.building_ref_revision()
    }

    /// Returns world-space triangle buffers for live flat building-site top surfaces.
    pub fn get_building_site_mesh_data_internal(&self) -> VarDictionary {
        let mut ground_vertices = Vec::with_capacity(self.allocator.building_sites.len() * 6);
        let mut asphalt_vertices = Vec::new();
        let mut concrete_vertices = Vec::new();

        for site in &self.allocator.building_sites {
            for (material, triangle) in site.foundation_mesh() {
                let target = match material {
                    None => &mut ground_vertices,
                    Some(SiteSurfaceMaterial::Asphalt) => &mut asphalt_vertices,
                    Some(SiteSurfaceMaterial::Concrete) => &mut concrete_vertices,
                };
                target.extend_from_slice(triangle);
            }
        }

        let mut dict = VarDictionary::new();
        dict.set(
            "revision",
            i64::try_from(self.allocator.building_ref_revision()).unwrap_or(i64::MAX),
        );
        dict.set(
            "ground_vertices",
            PackedVector3Array::from_iter(ground_vertices),
        );
        dict.set(
            "asphalt_vertices",
            PackedVector3Array::from_iter(asphalt_vertices),
        );
        dict.set(
            "concrete_vertices",
            PackedVector3Array::from_iter(concrete_vertices),
        );
        if building_site_debug_enabled() {
            dict.set("debug_enabled", true);
            dict.set("debug_sites", self.building_site_debug_sites());
        }
        dict
    }

    /// Returns the 12-float transforms for zone-filtered plot overlay MultiMeshes.
    pub fn get_building_plot_transforms_internal(&self, zone_type_int: u8) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if b.zone_type == target_zone {
                let world_x = b.center_x;
                let world_z = b.center_y;
                let world_y = b.support_height_m + 0.02; // Slightly above terrain

                let fd = b.facing_dir.normalized();
                let b_zx = fd.x;
                let b_zz = fd.y;
                let b_xx = fd.y;
                let b_xz = -fd.x;

                // Plot size is 10m * cell count (default 3x3 = 30x30)
                let cell_size = self.config.zone_cell_m;
                let sx = b.width_cells as f32 * cell_size;
                let sz = b.depth_cells as f32 * cell_size;
                let sy = 0.5; // Thin foundation box

                buffer.push(b_xx * sx);
                buffer.push(0.0);
                buffer.push(b_zx * sz);
                buffer.push(world_x);

                buffer.push(0.0);
                buffer.push(sy);
                buffer.push(0.0);
                buffer.push(world_y);

                buffer.push(b_xz * sx);
                buffer.push(0.0);
                buffer.push(b_zz * sz);
                buffer.push(world_z);
            }
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns neutral construction-site transforms for under-construction buildings.
    pub fn get_construction_site_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = b.support_height_m;
            let (right, front) = building_lot_basis(b);
            let (lot_width, lot_depth) = building_lot_size_m(self.config.zone_cell_m, b);
            let width = lot_width * SITE_INSET_SCALE;
            let depth = lot_depth * SITE_INSET_SCALE;
            let center_y = ground_y + SITE_PAD_HEIGHT_M * 0.5 + 0.015;

            push_oriented_box_transform(
                &mut buffer,
                b.center_x,
                b.center_y,
                center_y,
                right,
                front,
                width,
                SITE_PAD_HEIGHT_M,
                depth,
            );
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns raised foundation transforms for under-construction buildings.
    pub fn get_construction_foundation_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = b.support_height_m;
            let (right, front) = building_lot_basis(b);
            let (lot_width, lot_depth) = building_lot_size_m(self.config.zone_cell_m, b);
            let width = lot_width * FOUNDATION_INSET_SCALE;
            let depth = lot_depth * FOUNDATION_INSET_SCALE;
            let center_y = ground_y + SITE_PAD_HEIGHT_M + FOUNDATION_PAD_HEIGHT_M * 0.5 + 0.025;

            push_oriented_box_transform(
                &mut buffer,
                b.center_x,
                b.center_y,
                center_y,
                right,
                front,
                width,
                FOUNDATION_PAD_HEIGHT_M,
                depth,
            );
        }

        PackedFloat32Array::from_iter(buffer)
    }

    /// Returns procedural scaffold bar transforms for under-construction buildings.
    pub fn get_construction_scaffold_transforms_internal(
        &self,
        zone_type_int: u8,
    ) -> PackedFloat32Array {
        let target_zone = match zone_type_int {
            1 => ZoneType::Residential,
            2 => ZoneType::Commercial,
            3 => ZoneType::Industrial,
            _ => ZoneType::None,
        };

        if target_zone == ZoneType::None {
            return PackedFloat32Array::new();
        }

        let mut buffer = Vec::new();

        for b in &self.allocator.buildings {
            if !b.is_under_construction() || b.zone_type != target_zone {
                continue;
            }

            let ground_y = b.support_height_m;
            let (right, front) = building_lot_basis(b);
            let (lot_width, lot_depth) = building_lot_size_m(self.config.zone_cell_m, b);
            let width = lot_width * 0.82;
            let depth = lot_depth * 0.78;
            let height = construction_scaffold_height_m(b);

            push_scaffold_transforms(
                &mut buffer,
                b.center_x,
                b.center_y,
                ground_y,
                right,
                front,
                width,
                depth,
                height,
            );
        }

        PackedFloat32Array::from_iter(buffer)
    }

    fn operational_hour_fraction(&self) -> f32 {
        let seconds_per_minute = self.time.seconds_per_minute().max(f64::EPSILON);
        let minute_fraction = (self.time.time_elapsed / seconds_per_minute).clamp(0.0, 1.0);
        (f64::from(self.time.minute_of_day % 60) + minute_fraction) as f32 / 60.0
    }

    fn building_site_debug_sites(&self) -> VarArray {
        let mut sites = VarArray::new();
        for (site_idx, site) in self.allocator.building_sites.iter().enumerate() {
            let building = self.allocator.buildings.get(site_idx);
            let center = polygon_centroid(&site.footprint_world);
            let (min_x, min_z, max_x, max_z) = vector2_bounds(&site.footprint_world);
            let (lot_min_x, lot_min_z, lot_max_x, lot_max_z) =
                vector2_bounds(&site.lot_footprint_world);
            let mut dict = VarDictionary::new();
            dict.set("site_index", i64::try_from(site_idx).unwrap_or(i64::MAX));
            dict.set(
                "asset_id",
                building
                    .map(|b| b.asset_id.as_str())
                    .unwrap_or("<missing-building>"),
            );
            dict.set(
                "zone_type",
                building
                    .map(|b| format!("{:?}", b.zone_type))
                    .unwrap_or_else(|| "<missing-building>".to_string()),
            );
            dict.set("center", center);
            dict.set(
                "facing_dir",
                building.map(|b| b.facing_dir).unwrap_or(Vector2::ZERO),
            );
            dict.set(
                "edge_idx",
                i64::try_from(building.map(|b| b.edge_idx).unwrap_or(usize::MAX))
                    .unwrap_or(i64::MAX),
            );
            dict.set("side", i64::from(building.map(|b| b.side).unwrap_or(0)));
            dict.set(
                "frontage_t",
                building.map(|b| b.frontage_t).unwrap_or_default(),
            );
            dict.set(
                "side_offset_m",
                building.map(|b| b.side_offset).unwrap_or_default(),
            );
            dict.set(
                "width_cells",
                i64::from(building.map(|b| b.width_cells).unwrap_or_default()),
            );
            dict.set(
                "depth_cells",
                i64::from(building.map(|b| b.depth_cells).unwrap_or_default()),
            );
            dict.set(
                "lot_width_m",
                building
                    .map(|b| b.width_cells as f32 * self.config.zone_cell_m)
                    .unwrap_or(0.0),
            );
            dict.set(
                "lot_depth_m",
                building
                    .map(|b| b.depth_cells as f32 * self.config.zone_cell_m)
                    .unwrap_or(0.0),
            );
            dict.set("support_height_m", site.support_height_m);
            dict.set(
                "footprint_area_m2",
                polygon_signed_area(&site.footprint_world).abs(),
            );
            dict.set("bounds_min", Vector2::new(min_x, min_z));
            dict.set("bounds_max", Vector2::new(max_x, max_z));
            dict.set(
                "lot_area_m2",
                polygon_signed_area(&site.lot_footprint_world).abs(),
            );
            dict.set("lot_bounds_min", Vector2::new(lot_min_x, lot_min_z));
            dict.set("lot_bounds_max", Vector2::new(lot_max_x, lot_max_z));
            dict.set(
                "lot_footprint",
                PackedVector2Array::from_iter(site.lot_footprint_world.iter().copied()),
            );
            dict.set(
                "footprint",
                PackedVector2Array::from_iter(site.footprint_world.iter().copied()),
            );
            dict.set(
                "samples",
                self.building_site_debug_samples(site.support_height_m, &site.footprint_world),
            );
            let edge_debug = self.building_site_debug_edge_samples(
                site.support_height_m,
                &site.footprint_world,
                building,
            );
            dict.set(
                "road_sample_count",
                i64::try_from(edge_debug.road_sample_count).unwrap_or(i64::MAX),
            );
            dict.set(
                "road_height_min_m",
                edge_debug.road_height_min_m.unwrap_or(0.0),
            );
            dict.set(
                "road_height_max_m",
                edge_debug.road_height_max_m.unwrap_or(0.0),
            );
            dict.set("road_height_range_m", edge_debug.road_height_range_m());
            dict.set(
                "max_abs_support_delta_road_m",
                edge_debug.max_abs_support_delta_road_m,
            );
            dict.set(
                "terrain_visual_height_min_m",
                edge_debug.terrain_visual_height_min_m.unwrap_or(0.0),
            );
            dict.set(
                "terrain_visual_height_max_m",
                edge_debug.terrain_visual_height_max_m.unwrap_or(0.0),
            );
            dict.set(
                "terrain_visual_height_range_m",
                edge_debug.terrain_visual_height_range_m(),
            );
            dict.set(
                "max_abs_support_delta_visual_m",
                edge_debug.max_abs_support_delta_visual_m,
            );
            if let Some((point, road_height_m)) = edge_debug.claimed_road_sample {
                dict.set("claimed_road_probe_valid", true);
                dict.set("claimed_road_probe_point", point);
                dict.set("claimed_road_probe_has_height", road_height_m.is_some());
                dict.set("claimed_road_probe_height_m", road_height_m.unwrap_or(0.0));
                dict.set(
                    "claimed_road_probe_support_delta_m",
                    road_height_m
                        .map(|height_m| site.support_height_m - height_m)
                        .unwrap_or(0.0),
                );
            } else {
                dict.set("claimed_road_probe_valid", false);
                dict.set("claimed_road_probe_point", Vector2::ZERO);
                dict.set("claimed_road_probe_has_height", false);
                dict.set("claimed_road_probe_height_m", 0.0);
                dict.set("claimed_road_probe_support_delta_m", 0.0);
            }
            dict.set("edge_samples", edge_debug.samples);

            let mut surfaces = VarArray::new();
            for (surface_idx, surface) in site.surfaces.iter().enumerate() {
                let (surface_min_x, surface_min_z, surface_max_x, surface_max_z) =
                    vector2_bounds(&surface.vertices_world);
                let mut surface_dict = VarDictionary::new();
                surface_dict.set(
                    "surface_index",
                    i64::try_from(surface_idx).unwrap_or(i64::MAX),
                );
                surface_dict.set("name", surface.name.as_str());
                surface_dict.set("material", site_surface_material_label(surface.material));
                surface_dict.set("height_m", site.support_height_m);
                surface_dict.set(
                    "area_m2",
                    polygon_signed_area(&surface.vertices_world).abs(),
                );
                surface_dict.set("bounds_min", Vector2::new(surface_min_x, surface_min_z));
                surface_dict.set("bounds_max", Vector2::new(surface_max_x, surface_max_z));
                surface_dict.set(
                    "vertices",
                    PackedVector2Array::from_iter(surface.vertices_world.iter().copied()),
                );
                surfaces.push(&surface_dict.to_variant());
            }
            dict.set("surfaces", surfaces);
            sites.push(&dict.to_variant());
        }
        sites
    }

    fn building_site_debug_samples(
        &self,
        support_height_m: f32,
        footprint: &[Vector2],
    ) -> VarArray {
        let mut samples = VarArray::new();
        let center = polygon_centroid(footprint);
        push_site_debug_sample(&mut samples, "center", center, support_height_m, self);
        for (idx, point) in footprint.iter().copied().enumerate() {
            let label = format!("corner{}", idx);
            push_site_debug_sample(&mut samples, &label, point, support_height_m, self);
        }
        samples
    }

    fn building_site_debug_edge_samples(
        &self,
        support_height_m: f32,
        footprint: &[Vector2],
        building: Option<&Building>,
    ) -> BuildingSiteEdgeDebug {
        let mut edge_debug = BuildingSiteEdgeDebug::default();
        if footprint.len() < 2 {
            return edge_debug;
        }

        let center = polygon_centroid(footprint);
        let facing_dir = building
            .map(|building| normalized_or_zero(building.facing_dir))
            .unwrap_or(Vector2::ZERO);
        let roles = building_site_edge_roles(footprint, center, facing_dir);

        for edge_idx in 0..footprint.len() {
            let start = footprint[edge_idx];
            let end = footprint[(edge_idx + 1) % footprint.len()];
            let edge_vec = end - start;
            let length_m = edge_vec.length();
            let sample_count = ((length_m / BUILDING_SITE_DEBUG_EDGE_SAMPLE_STEP_M).ceil()
                as usize)
                .clamp(2, BUILDING_SITE_DEBUG_EDGE_SAMPLE_MAX);
            let role = roles
                .get(edge_idx)
                .copied()
                .unwrap_or(BuildingSiteDebugEdgeRole::Side);

            for sample_idx in 0..sample_count {
                let t = if sample_count > 1 {
                    sample_idx as f32 / (sample_count - 1) as f32
                } else {
                    0.0
                };
                let point = start.lerp(end, t);
                let requested_road_probe_point = if role == BuildingSiteDebugEdgeRole::Frontage {
                    point + facing_dir * BUILDING_SITE_DEBUG_ROAD_SAMPLE_INSET_M
                } else {
                    point
                };
                let road_sample = building_site_debug_visible_road_sample(
                    self,
                    requested_road_probe_point,
                    BUILDING_SITE_DEBUG_NEAREST_ROAD_SURFACE_RADIUS_M,
                );
                let road_probe_point = road_sample
                    .map(|(probe, _)| probe)
                    .unwrap_or(requested_road_probe_point);
                let road_height_m = road_sample.map(|(_, height_m)| height_m);
                let source_height_m =
                    self.heightmap.sample_height_world(point.x, point.y) * HEIGHT_SCALE;
                let visual_height_m =
                    self.heightmap.sample_visual_height_world(point.x, point.y) * HEIGHT_SCALE;

                edge_debug.record_terrain_visual_height(support_height_m, visual_height_m);
                if let Some(height_m) = road_height_m {
                    edge_debug.record_road_height(support_height_m, height_m);
                }

                let mut dict = VarDictionary::new();
                dict.set("edge_index", i64::try_from(edge_idx).unwrap_or(i64::MAX));
                dict.set("edge_role", role.label());
                dict.set(
                    "sample_index",
                    i64::try_from(sample_idx).unwrap_or(i64::MAX),
                );
                dict.set(
                    "sample_count",
                    i64::try_from(sample_count).unwrap_or(i64::MAX),
                );
                dict.set("t", t);
                dict.set("point", point);
                dict.set("road_probe_point", road_probe_point);
                dict.set("road_visible", road_height_m.is_some());
                dict.set("road_visible_height_m", road_height_m.unwrap_or(0.0));
                dict.set(
                    "support_delta_road_m",
                    road_height_m
                        .map(|height_m| support_height_m - height_m)
                        .unwrap_or(0.0),
                );
                dict.set("terrain_source_height_m", source_height_m);
                dict.set("terrain_visual_height_m", visual_height_m);
                dict.set("support_delta_source_m", support_height_m - source_height_m);
                dict.set("support_delta_visual_m", support_height_m - visual_height_m);
                edge_debug.samples.push(&dict.to_variant());
            }
        }

        if let Some(building) = building
            && building.edge_idx < self.region_graph.edge_count()
        {
            let edge = self.region_graph.edge(building.edge_idx);
            edge_debug.claimed_road_sample =
                claimed_road_side_connection_sample(self, edge, building).map(|sample| {
                    let road_height_m = self
                        .transit_network
                        .road_surface
                        .sample_visible_surface_height(
                            &self.region_graph,
                            &self.heightmap,
                            sample.x,
                            sample.y,
                        );
                    (sample, road_height_m)
                });
        }

        edge_debug
    }
}

#[derive(Default)]
struct BuildingSiteEdgeDebug {
    samples: VarArray,
    road_sample_count: usize,
    road_height_min_m: Option<f32>,
    road_height_max_m: Option<f32>,
    max_abs_support_delta_road_m: f32,
    terrain_visual_height_min_m: Option<f32>,
    terrain_visual_height_max_m: Option<f32>,
    max_abs_support_delta_visual_m: f32,
    claimed_road_sample: Option<(Vector2, Option<f32>)>,
}

impl BuildingSiteEdgeDebug {
    fn record_road_height(&mut self, support_height_m: f32, road_height_m: f32) {
        self.road_sample_count += 1;
        self.road_height_min_m = Some(
            self.road_height_min_m
                .map(|current| current.min(road_height_m))
                .unwrap_or(road_height_m),
        );
        self.road_height_max_m = Some(
            self.road_height_max_m
                .map(|current| current.max(road_height_m))
                .unwrap_or(road_height_m),
        );
        self.max_abs_support_delta_road_m = self
            .max_abs_support_delta_road_m
            .max((support_height_m - road_height_m).abs());
    }

    fn record_terrain_visual_height(&mut self, support_height_m: f32, visual_height_m: f32) {
        self.terrain_visual_height_min_m = Some(
            self.terrain_visual_height_min_m
                .map(|current| current.min(visual_height_m))
                .unwrap_or(visual_height_m),
        );
        self.terrain_visual_height_max_m = Some(
            self.terrain_visual_height_max_m
                .map(|current| current.max(visual_height_m))
                .unwrap_or(visual_height_m),
        );
        self.max_abs_support_delta_visual_m = self
            .max_abs_support_delta_visual_m
            .max((support_height_m - visual_height_m).abs());
    }

    fn road_height_range_m(&self) -> f32 {
        match (self.road_height_min_m, self.road_height_max_m) {
            (Some(min), Some(max)) => max - min,
            _ => 0.0,
        }
    }

    fn terrain_visual_height_range_m(&self) -> f32 {
        match (
            self.terrain_visual_height_min_m,
            self.terrain_visual_height_max_m,
        ) {
            (Some(min), Some(max)) => max - min,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildingSiteDebugEdgeRole {
    Frontage,
    Rear,
    Left,
    Right,
    Side,
}

impl BuildingSiteDebugEdgeRole {
    fn label(self) -> &'static str {
        match self {
            Self::Frontage => "frontage",
            Self::Rear => "rear",
            Self::Left => "left",
            Self::Right => "right",
            Self::Side => "side",
        }
    }
}

fn building_site_edge_roles(
    footprint: &[Vector2],
    center: Vector2,
    facing_dir: Vector2,
) -> Vec<BuildingSiteDebugEdgeRole> {
    let mut roles = vec![BuildingSiteDebugEdgeRole::Side; footprint.len()];
    if footprint.len() < 2 || facing_dir.length_squared() <= 1e-12 {
        return roles;
    }

    let mut frontage_idx = 0usize;
    let mut rear_idx = 0usize;
    let mut best_front_score = f32::NEG_INFINITY;
    let mut best_rear_score = f32::INFINITY;
    for edge_idx in 0..footprint.len() {
        let midpoint = (footprint[edge_idx] + footprint[(edge_idx + 1) % footprint.len()]) * 0.5;
        let score = (midpoint - center).dot(facing_dir);
        if score > best_front_score {
            best_front_score = score;
            frontage_idx = edge_idx;
        }
        if score < best_rear_score {
            best_rear_score = score;
            rear_idx = edge_idx;
        }
    }
    roles[frontage_idx] = BuildingSiteDebugEdgeRole::Frontage;
    roles[rear_idx] = BuildingSiteDebugEdgeRole::Rear;

    let right_dir = Vector2::new(facing_dir.y, -facing_dir.x);
    for edge_idx in 0..footprint.len() {
        if edge_idx == frontage_idx || edge_idx == rear_idx {
            continue;
        }
        let midpoint = (footprint[edge_idx] + footprint[(edge_idx + 1) % footprint.len()]) * 0.5;
        roles[edge_idx] = if (midpoint - center).dot(right_dir) >= 0.0 {
            BuildingSiteDebugEdgeRole::Right
        } else {
            BuildingSiteDebugEdgeRole::Left
        };
    }
    roles
}

fn claimed_road_side_connection_sample(
    core: &SimCore,
    edge: &Edge,
    building: &Building,
) -> Option<Vector2> {
    if edge.deleted || edge.physical_length <= 1e-6 || edge.physical_geometry.len() < 2 {
        return None;
    }
    let center =
        core.allocator
            .get_pos_on_edge(&core.region_graph, building.edge_idx, building.frontage_t);
    let tangent = core.allocator.get_tangent_on_edge(
        &core.region_graph,
        building.edge_idx,
        building.frontage_t,
    );
    if tangent.length_squared() <= 1e-12 {
        return None;
    }
    let normal = Vector2::new(tangent.y, -tangent.x) * building.side as f32;
    Some(center + normal * road_connection_lateral_offset_m(edge))
}

fn building_site_debug_visible_road_sample(
    core: &SimCore,
    point: Vector2,
    nearest_radius_m: f32,
) -> Option<(Vector2, f32)> {
    if let Some(height_m) = core
        .transit_network
        .road_surface
        .sample_visible_surface_height(&core.region_graph, &core.heightmap, point.x, point.y)
    {
        return Some((point, height_m));
    }
    let radius_m = nearest_radius_m.max(0.0);
    if radius_m <= f32::EPSILON {
        return None;
    }

    let mut candidates = core
        .region_graph
        .get_edges_near_point(Vector3::new(point.x, 0.0, point.y), radius_m);
    candidates.sort_unstable();
    candidates.dedup();

    let mut best: Option<(f32, usize, Vector2, f32)> = None;
    for edge_idx in candidates {
        let Some(edge) = core.region_graph.get_edge(edge_idx) else {
            continue;
        };
        if edge.deleted || edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
            continue;
        }
        let Some(projection) =
            BuildingAllocator::project_point_to_edge_centerline(edge_idx, edge, point)
        else {
            continue;
        };
        let center =
            BuildingAllocator::sample_pos_on_edge(&core.region_graph, edge_idx, projection.t);
        let tangent =
            BuildingAllocator::sample_tangent_on_edge(&core.region_graph, edge_idx, projection.t);
        if tangent.length_squared() <= 1e-12 {
            continue;
        }
        let normal = Vector2::new(tangent.y, -tangent.x) * projection.side as f32;
        let probe = center + normal * road_connection_lateral_offset_m(edge);
        let dist_sq = probe.distance_squared_to(point);
        if dist_sq > radius_m * radius_m {
            continue;
        }
        let Some(height_m) = core
            .transit_network
            .road_surface
            .sample_visible_surface_height(&core.region_graph, &core.heightmap, probe.x, probe.y)
        else {
            continue;
        };
        let replace = best
            .as_ref()
            .is_none_or(|(best_dist_sq, best_edge_idx, _, _)| {
                dist_sq
                    .total_cmp(best_dist_sq)
                    .then(edge_idx.cmp(best_edge_idx))
                    .is_lt()
            });
        if replace {
            best = Some((dist_sq, edge_idx, probe, height_m));
        }
    }
    best.map(|(_, _, probe, height_m)| (probe, height_m))
}

fn road_connection_lateral_offset_m(edge: &Edge) -> f32 {
    let sidewalk_m = if edge.primary_type == TransitType::Foot
        || (edge.allowed_types & TransitFlags::FOOT) == 0
    {
        0.0
    } else {
        SIDEWALK_WIDTH
    };
    (edge.width * 0.5 + sidewalk_m - BUILDING_SITE_DEBUG_ROAD_SAMPLE_INSET_M).max(0.0)
}

fn normalized_or_zero(value: Vector2) -> Vector2 {
    if value.length_squared() <= 1e-12 {
        Vector2::ZERO
    } else {
        value.normalized()
    }
}

fn building_site_debug_enabled() -> bool {
    let explicit_value = std::env::var("METRUM_DEBUG_BUILDINGS").unwrap_or_default();
    if explicit_value == "1" {
        return true;
    }
    let debug_value = std::env::var("METRUM_DEBUG").unwrap_or_default();
    if debug_value.is_empty() || debug_value == "0" {
        return false;
    }
    std::env::var("METRUM_DEBUG_FILTER")
        .unwrap_or_default()
        .split(',')
        .any(|entry| {
            let entry = entry.trim();
            entry == "buildings" || entry == "building-sites"
        })
}

fn push_site_debug_sample(
    samples: &mut VarArray,
    label: &str,
    point: Vector2,
    support_height_m: f32,
    core: &SimCore,
) {
    let source_height_m = core.heightmap.sample_height_world(point.x, point.y) * HEIGHT_SCALE;
    let visual_height_m =
        core.heightmap.sample_visual_height_world(point.x, point.y) * HEIGHT_SCALE;
    let mut dict = VarDictionary::new();
    dict.set("label", label);
    dict.set("point", point);
    dict.set("terrain_source_height_m", source_height_m);
    dict.set("terrain_visual_height_m", visual_height_m);
    dict.set("support_delta_source_m", support_height_m - source_height_m);
    dict.set("support_delta_visual_m", support_height_m - visual_height_m);
    samples.push(&dict.to_variant());
}

fn site_surface_material_label(material: SiteSurfaceMaterial) -> &'static str {
    match material {
        SiteSurfaceMaterial::Asphalt => "asphalt",
        SiteSurfaceMaterial::Concrete => "concrete",
    }
}

fn push_building_part_transform(
    buffer: &mut Vec<f32>,
    building: &Building,
    entry: Option<&AssetEntry>,
    part: &MeshPart,
    world_y: f32,
) {
    push_building_part_transform_for_pose(
        buffer,
        Vector2::new(building.center_x, building.center_y),
        world_y,
        building.facing_dir,
        entry,
        part,
    );
}

fn push_building_part_transform_for_pose(
    buffer: &mut Vec<f32>,
    center_2d: Vector2,
    world_y: f32,
    facing_dir: Vector2,
    entry: Option<&AssetEntry>,
    part: &MeshPart,
) {
    let (basis_x, basis_z) = building_local_xz_basis(facing_dir, building_frontage_forward(entry));
    let local = part.local_transform();
    let part_x_axis = basis_x * local.matrix3.x_axis.x + basis_z * local.matrix3.x_axis.z;
    let part_z_axis = basis_x * local.matrix3.z_axis.x + basis_z * local.matrix3.z_axis.z;
    let translation = center_2d + basis_x * local.translation.x + basis_z * local.translation.z;

    buffer.push(part_x_axis.x);
    buffer.push(0.0);
    buffer.push(part_z_axis.x);
    buffer.push(translation.x);

    buffer.push(0.0);
    buffer.push(local.matrix3.y_axis.y);
    buffer.push(0.0);
    buffer.push(world_y + local.translation.y);

    buffer.push(part_x_axis.y);
    buffer.push(0.0);
    buffer.push(part_z_axis.y);
    buffer.push(translation.y);
}

fn push_broken_building_transform(buffer: &mut Vec<f32>, building: &Building, world_y: f32) {
    let s = crate::config::BUILDING_VISUAL_SCALE;
    let front = if building.facing_dir.length_squared() > 1e-12 {
        building.facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    let right = Vector2::new(front.y, -front.x);

    buffer.push(right.x * s);
    buffer.push(0.0);
    buffer.push(front.x * s);
    buffer.push(building.center_x);

    buffer.push(0.0);
    buffer.push(s);
    buffer.push(0.0);
    buffer.push(world_y);

    buffer.push(right.y * s);
    buffer.push(0.0);
    buffer.push(front.y * s);
    buffer.push(building.center_y);
}

fn push_scaffold_transforms(
    buffer: &mut Vec<f32>,
    center_x: f32,
    center_z: f32,
    ground_y: f32,
    right: Vector2,
    front: Vector2,
    width: f32,
    depth: f32,
    height: f32,
) {
    let half_width = width * 0.5;
    let half_depth = depth * 0.5;
    let post_center_y = ground_y + height * 0.5 + 0.08;
    let post_offsets = [
        (-half_width, -half_depth),
        (0.0, -half_depth),
        (half_width, -half_depth),
        (-half_width, 0.0),
        (half_width, 0.0),
        (-half_width, half_depth),
        (0.0, half_depth),
        (half_width, half_depth),
    ];

    for (offset_x, offset_z) in post_offsets {
        let (x, z) = offset_point(center_x, center_z, right, front, offset_x, offset_z);
        push_oriented_box_transform(
            buffer,
            x,
            z,
            post_center_y,
            right,
            front,
            SCAFFOLD_POST_THICKNESS_M,
            height,
            SCAFFOLD_POST_THICKNESS_M,
        );
    }

    let rail_levels = [height * 0.42, height * 0.72, height * 0.94];
    for level in rail_levels {
        let rail_center_y = ground_y + level + 0.08;
        for offset_z in [-half_depth, half_depth] {
            let (x, z) = offset_point(center_x, center_z, right, front, 0.0, offset_z);
            push_oriented_box_transform(
                buffer,
                x,
                z,
                rail_center_y,
                right,
                front,
                width + SCAFFOLD_POST_THICKNESS_M,
                SCAFFOLD_RAIL_THICKNESS_M,
                SCAFFOLD_RAIL_THICKNESS_M,
            );
        }
        for offset_x in [-half_width, half_width] {
            let (x, z) = offset_point(center_x, center_z, right, front, offset_x, 0.0);
            push_oriented_box_transform(
                buffer,
                x,
                z,
                rail_center_y,
                right,
                front,
                SCAFFOLD_RAIL_THICKNESS_M,
                SCAFFOLD_RAIL_THICKNESS_M,
                depth + SCAFFOLD_POST_THICKNESS_M,
            );
        }
    }
}

fn push_oriented_box_transform(
    buffer: &mut Vec<f32>,
    center_x: f32,
    center_z: f32,
    center_y: f32,
    right: Vector2,
    front: Vector2,
    scale_x: f32,
    scale_y: f32,
    scale_z: f32,
) {
    buffer.push(right.x * scale_x);
    buffer.push(0.0);
    buffer.push(front.x * scale_z);
    buffer.push(center_x);

    buffer.push(0.0);
    buffer.push(scale_y);
    buffer.push(0.0);
    buffer.push(center_y);

    buffer.push(right.y * scale_x);
    buffer.push(0.0);
    buffer.push(front.y * scale_z);
    buffer.push(center_z);
}

fn polygon_signed_area(vertices: &[Vector2]) -> f32 {
    let mut area = 0.0;
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn polygon_centroid(vertices: &[Vector2]) -> Vector2 {
    if vertices.is_empty() {
        return Vector2::ZERO;
    }
    let mut sum = Vector2::ZERO;
    for vertex in vertices {
        sum += *vertex;
    }
    sum / vertices.len() as f32
}

fn vector2_bounds(vertices: &[Vector2]) -> (f32, f32, f32, f32) {
    let Some(first) = vertices.first().copied() else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let mut min_x = first.x;
    let mut min_z = first.y;
    let mut max_x = first.x;
    let mut max_z = first.y;
    for vertex in &vertices[1..] {
        min_x = min_x.min(vertex.x);
        min_z = min_z.min(vertex.y);
        max_x = max_x.max(vertex.x);
        max_z = max_z.max(vertex.y);
    }
    (min_x, min_z, max_x, max_z)
}

fn offset_point(
    center_x: f32,
    center_z: f32,
    right: Vector2,
    front: Vector2,
    offset_x: f32,
    offset_z: f32,
) -> (f32, f32) {
    (
        center_x + right.x * offset_x + front.x * offset_z,
        center_z + right.y * offset_x + front.y * offset_z,
    )
}

fn building_lot_basis(building: &Building) -> (Vector2, Vector2) {
    let front = if building.facing_dir.length_squared() > 1e-12 {
        building.facing_dir.normalized()
    } else {
        Vector2::new(0.0, 1.0)
    };
    let right = Vector2::new(front.y, -front.x);
    (right, front)
}

fn building_lot_size_m(cell_size_m: f32, building: &Building) -> (f32, f32) {
    (
        building.width_cells as f32 * cell_size_m,
        building.depth_cells as f32 * cell_size_m,
    )
}

fn construction_rise_offset_m(building: &Building, progress: f32) -> f32 {
    let t = progress.clamp(0.0, 1.0);
    (1.0 - t) * construction_initial_rise_offset_m(building)
}

fn construction_initial_rise_offset_m(building: &Building) -> f32 {
    construction_initial_rise_offset_for_height_m(construction_scaffold_height_m(building))
}

fn construction_initial_rise_offset_for_height_m(scaffold_height_m: f32) -> f32 {
    (scaffold_height_m.max(0.0) * CONSTRUCTION_RISE_INITIAL_HEIGHT_FACTOR
        + CONSTRUCTION_RISE_INITIAL_EXTRA_M)
        .clamp(
            CONSTRUCTION_RISE_INITIAL_MIN_OFFSET_M,
            CONSTRUCTION_RISE_INITIAL_MAX_OFFSET_M,
        )
}

fn construction_visual_progress(building: &Building, operational_hour_fraction: f32) -> f32 {
    construction_visual_progress_from_hours(
        building.construction_total_hours,
        building.construction_remaining_hours,
        operational_hour_fraction,
    )
}

fn construction_visual_progress_from_hours(
    total_hours: u16,
    remaining_hours: u16,
    operational_hour_fraction: f32,
) -> f32 {
    if total_hours == 0 {
        return 1.0;
    }
    let total = f32::from(total_hours);
    let remaining = f32::from(remaining_hours.min(total_hours));
    let completed_whole_hours = (total - remaining).max(0.0);
    ((completed_whole_hours + operational_hour_fraction.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0)
}

fn construction_scaffold_height_m(building: &Building) -> f32 {
    let level = f32::from(building.level.max(1));
    match building.zone_type {
        ZoneType::Residential => 7.0 + level * 2.2,
        ZoneType::Commercial => 8.0 + level * 3.0,
        ZoneType::Industrial => 8.5 + level * 2.8,
        ZoneType::Office | ZoneType::Mixed | ZoneType::None => 7.0 + level * 2.5,
    }
    .clamp(7.0, 18.0)
}

fn building_frontage_forward(entry: Option<&AssetEntry>) -> [f32; 3] {
    entry
        .map(|entry| entry.manifest.building_frontage_forward())
        .unwrap_or([0.0, 0.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn building_part_pose_matches_editor_yaw_scale_and_pivot() {
        for yaw in [-90.0_f32, -31.0, 0.0, 47.0, 90.0] {
            for heading in [-2.4_f32, 0.0, 1.2, std::f32::consts::PI] {
                let mut part = MeshPart::single_lod0("rotated", "unused.glb");
                part.position = [2.0, 3.0, -1.0];
                part.rotation_degrees = [0.0, yaw, 0.0];
                part.scale = 2.0;
                part.pivot_offset = Some([0.5, 0.25, -0.75]);
                let mut buffer = Vec::new();
                let origin = Vector3::new(11.0, 17.0, -5.0);
                push_building_part_transform_for_pose(
                    &mut buffer,
                    Vector2::new(origin.x, origin.z),
                    origin.y,
                    Vector2::new(heading.sin(), heading.cos()),
                    None,
                    &part,
                );
                let actual = Transform3D::new(
                    Basis::from_cols(
                        Vector3::new(buffer[0], buffer[4], buffer[8]),
                        Vector3::new(buffer[1], buffer[5], buffer[9]),
                        Vector3::new(buffer[2], buffer[6], buffer[10]),
                    ),
                    Vector3::new(buffer[3], buffer[7], buffer[11]),
                );
                // Independent oracle: the editor uses Godot's positive-Y Basis, not our helper.
                let building = Basis::from_axis_angle(Vector3::UP, heading);
                let rotation = Basis::from_axis_angle(Vector3::UP, yaw.to_radians());
                for point in [Vector3::ZERO, Vector3::RIGHT, Vector3::UP, Vector3::BACK] {
                    let expected = origin
                        + building
                            * (Vector3::new(2.0, 3.0, -1.0)
                                + rotation * ((point + Vector3::new(0.5, 0.25, -0.75)) * 2.0));
                    assert!((actual * point).distance_to(expected) < 1e-5);
                }
            }
        }
    }

    fn assert_vec2_close(actual: Vector2, expected: Vector2) {
        assert!(
            (actual.x - expected.x).abs() < 1e-5 && (actual.y - expected.y).abs() < 1e-5,
            "expected ({:.3}, {:.3}), got ({:.3}, {:.3})",
            expected.x,
            expected.y,
            actual.x,
            actual.y
        );
    }

    #[test]
    fn test_building_basis_aligns_authored_front_to_road_facing_dir() {
        let world_front = Vector2::new(0.0, -1.0);

        let (basis_x, basis_z) = building_local_xz_basis(world_front, [0.0, 0.0, 1.0]);
        assert_vec2_close(basis_x * 0.0 + basis_z * 1.0, world_front);

        let (basis_x, basis_z) = building_local_xz_basis(world_front, [0.0, 0.0, -1.0]);
        assert_vec2_close(basis_x * 0.0 + basis_z * -1.0, world_front);
    }

    #[test]
    fn construction_visual_progress_interpolates_between_hour_ticks() {
        assert!((construction_visual_progress_from_hours(4, 4, 0.0) - 0.0).abs() < 1e-6);
        assert!((construction_visual_progress_from_hours(4, 4, 0.5) - 0.125).abs() < 1e-6);
        assert!((construction_visual_progress_from_hours(4, 3, 0.5) - 0.375).abs() < 1e-6);
        assert!((construction_visual_progress_from_hours(4, 1, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn construction_initial_rise_offset_starts_near_surface() {
        assert!(
            (construction_initial_rise_offset_for_height_m(7.0) - 6.55).abs() < 1e-5,
            "low-rise construction should become visible quickly"
        );
        assert_eq!(
            construction_initial_rise_offset_for_height_m(18.0),
            CONSTRUCTION_RISE_INITIAL_MAX_OFFSET_M,
            "tall buildings should not start deeply buried"
        );
        assert_eq!(
            construction_initial_rise_offset_for_height_m(0.0),
            CONSTRUCTION_RISE_INITIAL_MIN_OFFSET_M
        );
    }
}
