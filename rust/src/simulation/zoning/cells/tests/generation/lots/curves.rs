// SPDX-License-Identifier: GPL-2.0-only

//! Retained curved frontage through graph subdivision and persistence.

use super::*;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::{TransitNetwork, topology};

mod performance;

fn curved_fixture() -> (RegionGraph, ZoningSystem) {
    curved_fixture_at(DVec2::ZERO, false)
}

fn curved_fixture_at(offset: DVec2, reversed: bool) -> (RegionGraph, ZoningSystem) {
    let mut graph = RegionGraph::new();
    let mut points: Vec<_> = (0..=24)
        .map(|i| {
            let x = i as f32 * 10.0;
            Vector3::new(x + offset.x as f32, 0.0, x * x * 0.001 + offset.y as f32)
        })
        .collect();
    if reversed {
        points.reverse();
    }
    let a = node(&mut graph, points[0].x, points[0].z);
    let b = node(&mut graph, points[24].x, points[24].z);
    polyline(&mut graph, a, b, points);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let bounds = curve_bounds(offset);
    zoning.generate_cells(&graph, bounds, |_| false);
    let mut cells = Vec::new();
    zoning.cells.visit_in_bounds(bounds, |key| cells.push(key));
    cells.sort_unstable();
    let paint = CellSelection {
        revision: zoning.cells.revision(),
        cells,
    };
    assert!(paint.cells.len() > 150, "{} cells", paint.cells.len());
    zoning.paint_cells(&paint, 1, |_| false).unwrap();
    (graph, zoning)
}

fn curve_bounds(offset: DVec2) -> CellBounds {
    CellBounds {
        min: extent().min + offset,
        max: extent().max + offset,
    }
}

#[test]
fn curve_guides_round_trip_fractional_widths_and_all_quadrants() {
    let (template, _) = curved_fixture();
    for width in [3.7_f32, 7.1, 11.333333] {
        for angle in [0.0_f64, 0.5, 1.8, 3.4, 5.3] {
            let u = DVec2::new(angle.cos(), angle.sin());
            let v = u.perp();
            let points: Vec<_> = template
                .edge(0)
                .physical_geometry
                .iter()
                .map(|p| {
                    let p = u * f64::from(p.x) + v * f64::from(p.z);
                    Vector3::new(p.x as f32, 0.0, p.y as f32)
                })
                .collect();
            let mut graph = RegionGraph::new();
            let a = node(&mut graph, points[0].x, points[0].z);
            let b = node(
                &mut graph,
                points[points.len() - 1].x,
                points[points.len() - 1].z,
            );
            let edge = polyline(&mut graph, a, b, points);
            graph.edge_mut(edge).width = width;
            let mut zoning = ZoningSystem::new(&WorldConfig::default());
            zoning.generate_cells(&graph, extent(), |_| false);
            let guides = zoning.cells.curve_sources_in_bounds(extent());
            assert!(!guides.is_empty());
            for source in guides {
                assert!(source.is_valid(), "width={width} angle={angle}: {source:?}");
                let json = serde_json::to_string(source.as_ref()).unwrap();
                let decoded: crate::simulation::zoning::cells::CellCurveSource =
                    serde_json::from_str(&json).unwrap();
                assert_eq!(source.as_ref(), &decoded);
                assert!(decoded.is_valid());
            }
        }
    }
}

fn split_curve(graph: &mut RegionGraph, zoning: &mut ZoningSystem) {
    let split = graph.edge(0).physical_geometry[6].lerp(graph.edge(0).physical_geometry[7], 0.3);
    let junction = node(graph, split.x, split.z);
    topology::split_edge(
        &mut TransitNetwork::new(),
        graph,
        0,
        6,
        0.3,
        junction,
        zoning,
        &mut BuildingAllocator::new(),
    );
}

#[test]
fn curved_split_preserves_painted_frontages_and_existing_lots() {
    for occupied in [false, true] {
        let (mut graph, mut zoning) = curved_fixture();
        let shape = CellLotSize {
            profile: 1,
            width: 2,
            depth: 2,
        };
        let created = zoning
            .derive_cell_lots(&graph, extent(), &[shape], |_, _| false)
            .created;
        assert!(!created.is_empty());
        if occupied {
            for (index, id) in created.iter().enumerate() {
                assert!(zoning.occupy_parcel(id.raw(), index));
            }
        }
        let paint = zoning.cells.saved_cells();
        let frontage: Vec<_> = paint
            .iter()
            .filter_map(|&(key, _)| (!zoning.cells.frontages(key).is_empty()).then_some(key))
            .collect();
        assert!(frontage.len() > 10);
        let original: Vec<_> = created
            .iter()
            .map(|&id| {
                let lot = zoning.parcels.get(id).unwrap();
                (
                    id,
                    lot.corners(),
                    lot.build_generation(),
                    lot.occupied_building(),
                )
            })
            .collect();
        split_curve(&mut graph, &mut zoning);
        zoning.generate_cells(&graph, extent(), |_| false);
        assert_eq!(zoning.cells.saved_cells(), paint);
        assert!(
            frontage
                .iter()
                .all(|&key| !zoning.cells.frontages(key).is_empty()),
            "curved split lost existing frontage"
        );
        let report = zoning.derive_cell_lots(&graph, extent(), &[shape], |_, _| false);
        assert!(report.retired.is_empty());
        assert!(report.created.is_empty());
        for (id, corners, generation, occupant) in original {
            let lot = zoning.parcels.get(id).unwrap();
            assert_eq!(lot.corners(), corners);
            assert_eq!(lot.build_generation(), generation);
            assert_eq!(lot.occupied_building(), occupant);
        }
        assert_disjoint(&zoning.cells, extent());
    }
}

#[test]
fn curved_sources_round_trip_before_split_and_restored_geometry() {
    for offset in [DVec2::ZERO, DVec2::new(-4000.0, 5000.0)] {
        for reversed in [false, true] {
            let (mut graph, mut zoning) = curved_fixture_at(offset, reversed);
            let original = graph.clone();
            let bounds = curve_bounds(offset);
            let paint = zoning.cells.saved_cells();
            let frontages: Vec<_> = paint
                .iter()
                .filter_map(|&(key, _)| (!zoning.cells.frontages(key).is_empty()).then_some(key))
                .collect();
            let mut restored = CellStore::default();
            for (id, frame) in zoning.cells.saved_frames() {
                assert!(restored.restore_saved_frame(id, frame));
            }
            for &(key, profile) in &paint {
                assert!(restored.restore_saved_cell(key, profile));
            }
            let sources = zoning.cells.saved_curve_sources();
            assert!(sources.len() > 2);
            for source in sources {
                let json = serde_json::to_string(source).unwrap();
                let decoded = serde_json::from_str(&json).unwrap();
                assert_eq!(source, &decoded);
                assert!(restored.restore_curve_source(decoded));
            }
            zoning.cells = restored;
            zoning.generate_cells(&graph, bounds, |_| false);
            let shape = CellLotSize {
                profile: 1,
                width: 2,
                depth: 2,
            };
            let lots = zoning
                .derive_cell_lots(&graph, bounds, &[shape], |_, _| false)
                .created;
            assert!(!lots.is_empty());
            split_curve(&mut graph, &mut zoning);
            for current in [&graph, &original] {
                zoning.generate_cells(current, bounds, |_| false);
                assert_eq!(zoning.cells.saved_cells(), paint);
                for &key in &frontages {
                    assert!(
                        !zoning.cells.frontages(key).is_empty(),
                        "lost {key:?}, offset={offset:?}, reversed={reversed}"
                    );
                }
                let report = zoning.derive_cell_lots(current, bounds, &[shape], |_, _| false);
                assert!(
                    report.retired.is_empty() && report.created.is_empty(),
                    "changed lots at {offset:?}, reversed={reversed}: {report:?}"
                );
                assert_disjoint(&zoning.cells, bounds);
            }
        }
    }
}

#[test]
fn reversing_current_curve_keeps_existing_lots_and_flips_attachment_side() {
    let (graph, mut zoning) = curved_fixture();
    let shape = CellLotSize {
        profile: 1,
        width: 2,
        depth: 2,
    };
    let ids = zoning
        .derive_cell_lots(&graph, extent(), &[shape], |_, _| false)
        .created;
    let lots: Vec<_> = ids
        .iter()
        .map(|&id| zoning.parcels.get(id).unwrap().clone())
        .collect();
    assert!(!lots.is_empty());
    let mut reversed = RegionGraph::new();
    let mut points = graph.edge(0).physical_geometry.clone();
    points.reverse();
    let a = node(&mut reversed, points[0].x, points[0].z);
    let b = node(
        &mut reversed,
        points[points.len() - 1].x,
        points[points.len() - 1].z,
    );
    polyline(&mut reversed, a, b, points);
    zoning.generate_cells(&reversed, extent(), |_| false);
    let report = zoning.derive_cell_lots(&reversed, extent(), &[shape], |_, _| false);
    assert!(report.retired.is_empty() && report.created.is_empty());
    for before in lots {
        let after = zoning.parcels.get(before.id()).unwrap();
        assert_eq!(before.corners(), after.corners());
        assert_eq!(before.side(), -after.side());
        assert!((before.frontage_center_t() + after.frontage_center_t() - 1.0).abs() < 1e-6);
    }
}

#[test]
fn erased_curved_sources_restore_after_empty_cache_eviction() {
    let (mut graph, mut zoning) = curved_fixture();
    let paint = zoning.cells.saved_cells();
    let source_count = zoning.cells.saved_curve_sources().len();
    let selected = CellSelection {
        revision: zoning.cells.revision(),
        cells: paint.iter().map(|&(key, _)| key).collect(),
    };
    let edit = zoning.cells.paint(&selected, 0).unwrap();
    split_curve(&mut graph, &mut zoning);
    zoning.generate_cells(&graph, extent(), |_| false);
    assert!(zoning.cells.saved_curve_sources().is_empty());
    assert!(zoning.cells.restore_paint_values(&edit));
    assert_eq!(zoning.cells.saved_cells(), paint);
    assert_eq!(zoning.cells.saved_curve_sources().len(), source_count);
    zoning.generate_cells(&graph, extent(), |_| false);
    assert!(
        !zoning
            .derive_cell_lots(&graph, extent(), &[size()], |_, _| false)
            .created
            .is_empty()
    );
}

#[test]
fn changed_curved_geometry_cannot_inherit_old_source_frontage() {
    let (graph, mut zoning) = curved_fixture();
    let paint = zoning.cells.saved_cells();
    let mut changed = RegionGraph::new();
    let points: Vec<_> = graph
        .edge(0)
        .physical_geometry
        .iter()
        .map(|p| *p + Vector3::new(0.0, 0.0, 0.1))
        .collect();
    let a = node(&mut changed, points[0].x, points[0].z);
    let b = node(
        &mut changed,
        points[points.len() - 1].x,
        points[points.len() - 1].z,
    );
    polyline(&mut changed, a, b, points);
    zoning.generate_cells(&changed, extent(), |_| false);
    assert_eq!(zoning.cells.saved_cells(), paint);
    assert!(
        paint
            .iter()
            .all(|&(key, _)| zoning.cells.frontages(key).is_empty())
    );
    assert!(
        zoning
            .derive_cell_lots(&changed, extent(), &[size()], |_, _| false)
            .created
            .is_empty()
    );
}
