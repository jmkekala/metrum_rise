// SPDX-License-Identifier: GPL-2.0-only

//! Pure flat-lot yard planning for authoring, using the spawn path's planting and hedge kernels.
//! Scratch edits reuse vegetation's cell index; no sandbox or document state is changed.

use super::*;
use crate::assets::asset::{SiteSurfaceMaterial, YardHedgeKind, YardPlantKind};
use crate::simulation::buildings::allocator::yard::YardPlantingArea;
use crate::simulation::buildings::allocator::{BuildingSiteClient, BuildingSiteSurfaceClient};
use crate::simulation::vegetation::edits::VegetationEdits;
use placement::HedgeSite;
use serde_json::Value;

struct FlatYard {
    site: BuildingSiteClient,
    edits: VegetationEdits,
}

impl HedgeSite for FlatYard {
    fn hedge_grid(&self) -> (f32, u32) {
        // The default canopy owner grid; named hedge variants do not depend on its salt.
        (16.0, 0)
    }

    fn hedge_cell(&self, cell: VegetationCell) -> &[Plant] {
        self.edits.cell(cell).1
    }

    fn hedge_clear(&self, plant: &Plant) -> bool {
        let class = brush::PlantClass::of(plant.species, plant.variant);
        let radius = placement::authored_radius(
            class,
            self.site
                .contains_yard_planting(Vector2::new(plant.x, plant.z)),
        );
        // Identical nine samples and wall/paving predicate to a flat developed game site.
        clear_footprint(plant.x, plant.z, 0.0, radius, 0.0, |x, z| {
            let p = Vector2::new(x, z);
            (!self.site.contains_point(p) || !self.site.blocks_yard_plant(p)).then_some(0.0)
        })
    }
}

impl FlatYard {
    fn add(&mut self, plant: Plant) {
        self.edits.add(
            cell_at(
                Vector2::new(plant.x, plant.z),
                VegetationLayer::Canopy,
                self.hedge_grid().0,
            ),
            plant,
        );
    }

    fn occupied(&self, plant: &Plant) -> bool {
        let radius = brush::PlantClass::of(plant.species, plant.variant).spacing();
        let pos = Vector2::new(plant.x, plant.z);
        let first = cell_at(
            pos - Vector2::splat(radius),
            VegetationLayer::Canopy,
            self.hedge_grid().0,
        );
        let last = cell_at(
            pos + Vector2::splat(radius),
            VegetationLayer::Canopy,
            self.hedge_grid().0,
        );
        let conflicts = placement::plant_conflicts(plant);
        (first.z..=last.z).any(|z| {
            (first.x..=last.x).any(|x| {
                self.hedge_cell(VegetationCell {
                    layer: VegetationLayer::Canopy,
                    x,
                    z,
                })
                .iter()
                .any(conflicts)
            })
        })
    }
}

/// Plan one isolated, flat, developed lot from editor parameters and loaded wall bounds.
/// The synthetic parcel salt uses the same area seeding as a spawned building. Work is local:
/// O(V² + L × V + C × (V + P)) for lot vertices V, hedge length L, candidates C
/// and accepted area plants P, plus bounded indexed neighbor checks; never city size.
pub(crate) fn plan(data: &Value, structures: &[[[f32; 2]; 2]], salt: u64) -> Vec<Plant> {
    let number = |v: &Value| v.as_f64().unwrap_or(0.0) as f32;
    let polygon = |v: &Value| -> Vec<Vector2> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|p| Vector2::new(number(&p[0]), number(&p[1])))
            .collect()
    };
    let w = number(&data["lot_width_cells"]) * 10.0 * 0.5;
    let d = number(&data["lot_depth_cells"]) * 10.0 * 0.5;
    let mut planting = Vec::new();
    let mut surfaces = Vec::new();
    for surface in data["site_surfaces"].as_array().into_iter().flatten() {
        let vertices = polygon(&surface["vertices"]);
        let points: Vec<_> = vertices.iter().map(|p| [p.x, p.y]).collect();
        // Incomplete polygons are normal while editing. Do not scan an invalid or unbounded grid.
        if !crate::assets::asset::geometry::is_valid(&points)
            || vertices.iter().any(|p| p.x.abs() > w || p.y.abs() > d)
        {
            continue;
        }
        match YardPlantKind::from_name(surface["material"].as_str().unwrap_or_default()) {
            Some(plants) => planting.push(YardPlantingArea {
                plants,
                polygon: points,
                origin: Vector2::ZERO,
                basis_x: Vector2::RIGHT,
                basis_z: Vector2::DOWN,
            }),
            None => surfaces.push(BuildingSiteSurfaceClient {
                material: SiteSurfaceMaterial::Asphalt,
                name: String::new(),
                vertices_world: vertices,
            }),
        }
    }
    let footprint = [
        Vector2::new(-w, -d),
        Vector2::new(w, -d),
        Vector2::new(w, d),
        Vector2::new(-w, d),
    ];
    let mut yard = FlatYard {
        site: BuildingSiteClient {
            structure_world: structures
                .iter()
                .map(|[a, b]| {
                    [
                        Vector2::new(a[0], a[1]),
                        Vector2::new(b[0], a[1]),
                        Vector2::new(b[0], b[1]),
                        Vector2::new(a[0], b[1]),
                    ]
                })
                .collect(),
            planting_world: planting
                .iter()
                .map(|area| {
                    area.polygon
                        .iter()
                        .map(|p| Vector2::new(p[0], p[1]))
                        .collect()
                })
                .collect(),
            footprint_world: footprint.to_vec(),
            lot_footprint_world: footprint,
            support_height_m: 0.0,
            surfaces,
        },
        edits: VegetationEdits::default(),
    };
    let mut result = Vec::new();
    if let Some(preset) = YardHedgeKind::from_name(data["yard_hedge"].as_str().unwrap_or_default())
        .and_then(|hedge| preset(brush::HEDGE_LOW_PRESET + hedge.index() as i64))
    {
        let rows = crate::assets::authoring::yard_hedge_rows(data, structures);
        for row in rows.as_array().into_iter().flatten() {
            let point = |v: &Value| Vector2::new(number(&v[0]), number(&v[1]));
            let modules = placement::plan_row(
                &yard,
                preset,
                point(&row["from"]),
                point(&row["to"]),
                [
                    row["join_from"].as_bool().unwrap_or(false),
                    row["join_to"].as_bool().unwrap_or(false),
                ],
                Some(placement::YARD_SHARE_M),
            );
            for (_, plant) in modules {
                yard.add(plant);
                result.push(plant);
            }
        }
    }
    for (index, area) in planting.iter().enumerate() {
        result.extend(yard_planting::plan_area((salt, 0), index, area, |plant| {
            if !yard.hedge_clear(&plant) || yard.occupied(&plant) {
                return None;
            }
            yard.add(plant);
            Some(plant)
        }));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (Value, [[[f32; 2]; 2]; 1]) {
        (
            json!({"asset_class": "building", "zone_type": "residential", "lot_width_cells": 4, "lot_depth_cells": 4,
            "yard_hedge": "medium", "frontage_forward": [0, 0, 1],
            "site_surfaces": [
                {"material": "mixed", "vertices": [[-18,-18],[18,-18],[18,-4],[-18,-4]]},
                {"material": "flowers", "vertices": [[-18,4],[18,4],[18,18],[-18,18]]},
                {"material": "concrete", "vertices": [[-2,0],[2,0],[2,20],[-2,20]]}]}),
            [[[-3.0, -3.0], [3.0, 3.0]]],
        )
    }

    #[test]
    fn flat_yard_is_repeatable_rerolls_and_keeps_walls_and_paving_clear() {
        let (data, walls) = fixture();
        let first = plan(&data, &walls, 1);
        assert_eq!(first, plan(&data, &walls, 1));
        assert_ne!(first, plan(&data, &walls, 2));
        assert!(
            first
                .iter()
                .any(|p| brush::hedge_index(p.species, p.variant).is_some())
        );
        assert!(first.iter().any(|p| p.species < 2));
        assert!(
            first
                .iter()
                .any(|p| p.species == 2 && brush::hedge_index(p.species, p.variant).is_none())
        );
        assert!(first.iter().all(|p| !(p.x.abs() < 3.0 && p.z.abs() < 3.0)));
        assert!(
            first
                .iter()
                .all(|p| !(p.x.abs() < 2.0 && (0.0..20.0).contains(&p.z)))
        );
        assert!(plan(&json!({}), &[], 1).is_empty());
    }

    #[test]
    fn planting_kernel_uses_parcel_generation_area_and_lot_frame() {
        let area = YardPlantingArea {
            plants: YardPlantKind::Flowers,
            polygon: vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]],
            origin: Vector2::ZERO,
            basis_x: Vector2::RIGHT,
            basis_z: Vector2::DOWN,
        };
        let plants = yard_planting::plan_area((23, 4), 0, &area, Some);
        assert!(!plants.is_empty());
        assert_eq!(plants, yard_planting::plan_area((23, 4), 0, &area, Some));
        for (key, index) in [((24, 4), 0), ((23, 5), 0), ((23, 4), 1)] {
            assert_ne!(plants, yard_planting::plan_area(key, index, &area, Some));
        }
        assert!(yard_planting::plan_area::<Plant>((23, 4), 0, &area, |_| None).is_empty());
        let moved = YardPlantingArea {
            origin: Vector2::new(100.0, 200.0),
            ..area
        };
        let translated = yard_planting::plan_area((23, 4), 0, &moved, Some);
        assert_eq!(plants.len(), translated.len());
        for (a, b) in plants.iter().zip(translated) {
            assert_eq!(
                (
                    a.x + 100.0,
                    a.z + 200.0,
                    a.yaw,
                    a.scale,
                    a.species,
                    a.variant
                ),
                (b.x, b.z, b.yaw, b.scale, b.species, b.variant)
            );
        }
    }

    #[test]
    #[ignore = "release timing probe; run explicitly with --release --ignored --nocapture"]
    fn yard_preview_planning_timing() {
        let (data, walls) = fixture();
        let start = std::time::Instant::now();
        let mut count = 0;
        for seed in 0..1000 {
            count += std::hint::black_box(plan(&data, &walls, seed)).len();
        }
        eprintln!(
            "1000 flat 40x40 m yards: {:?}, {count} placements",
            start.elapsed()
        );
    }
}
