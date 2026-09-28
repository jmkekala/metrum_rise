// SPDX-License-Identifier: GPL-2.0-only

//! Matched local curve refresh cost with distant curved paint and occupied lots.

use super::*;
use std::hint::black_box;
use std::time::Instant;

#[test]
#[ignore = "matched release curve locality measurement; excludes background and graph setup"]
fn benchmark_cell_curve_frontage_locality() {
    let mut baseline = None;
    for remote in [0, 128, 1024] {
        let (mut graph, mut zoning) = curved_fixture();
        let original_edge = graph.edge(0).clone();
        let shape = CellLotSize {
            profile: 1,
            width: 2,
            depth: 2,
        };
        let local_lots = zoning
            .derive_cell_lots(&graph, extent(), &[shape], |_, _| false)
            .created;
        assert!(!local_lots.is_empty());
        let mut occupant = 0;
        for &id in &local_lots {
            assert!(zoning.occupy_parcel(id.raw(), occupant));
            occupant += 1;
        }
        split_curve(&mut graph, &mut zoning);
        for index in 0..remote {
            let origin = DVec2::new(
                2000.0 + (index % 32) as f64 * 220.0,
                2000.0 + (index / 32) as f64 * 220.0,
            );
            let points: Vec<_> = [(0.0, 0.0), (30.0, 2.0), (60.0, 8.0)]
                .into_iter()
                .map(|(x, y)| Vector3::new((origin.x + x) as f32, 0.0, (origin.y + y) as f32))
                .collect();
            let a = node(&mut graph, points[0].x, points[0].z);
            let b = node(&mut graph, points[2].x, points[2].z);
            polyline(&mut graph, a, b, points);
            let bounds = CellBounds {
                min: origin - DVec2::splat(80.0),
                max: origin + DVec2::new(140.0, 100.0),
            };
            zoning.generate_cells(&graph, bounds, |_| false);
            let selection = CellSelection {
                revision: zoning.cells.revision(),
                cells: keys(&zoning.cells, bounds),
            };
            zoning.paint_cells(&selection, 1, |_| false).unwrap();
            for id in zoning
                .derive_cell_lots(&graph, bounds, &[shape], |_, _| false)
                .created
            {
                assert!(zoning.occupy_parcel(id.raw(), occupant));
                occupant += 1;
            }
        }
        let mut restored = graph.clone();
        restored.remove_from_spatial_index(0);
        restored.remove_from_spatial_index(1);
        *restored.edge_mut(0) = original_edge;
        restored.edge_mut(1).deleted = true;
        restored.add_to_spatial_index(0);
        restored.rebuild_adjacency_list();
        let graphs = [&graph, &restored];
        let expected: Vec<_> = graphs
            .iter()
            .map(|g| zoning.generate_cells(g, extent(), |_| false))
            .collect();
        let mut products: Vec<_> = keys(&zoning.cells, extent())
            .into_iter()
            .map(|key| (zoning.cells.frame(key.grid).unwrap(), key.x, key.y))
            .collect();
        products.sort_unstable();
        if let Some((prior_reports, prior_keys)) = &baseline {
            assert_eq!(&expected, prior_reports);
            assert_eq!(&products, prior_keys);
        } else {
            baseline = Some((expected.clone(), products));
        }
        let run = |zoning: &mut ZoningSystem| {
            for (g, expected) in graphs.iter().zip(&expected) {
                let report = zoning.generate_cells(g, extent(), |_| false);
                let lots = zoning.derive_cell_lots(g, extent(), &[shape], |_, _| false);
                assert_eq!(report, *expected);
                assert!(lots.created.is_empty() && lots.retired.is_empty());
                assert_eq!(lots.reattached.len(), local_lots.len());
                black_box(report);
            }
        };
        zoning.derive_cell_lots(&restored, extent(), &[shape], |_, _| false);
        for _ in 0..8 {
            run(&mut zoning);
        }
        let mut samples = [0.0; 21];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..8 {
                run(black_box(&mut zoning));
            }
            *sample = start.elapsed().as_secs_f64() * 1e6 / 16.0;
        }
        samples.sort_unstable_by(f64::total_cmp);
        println!(
            "remote_curves={remote} occupied_lots={occupant} reserved_cells={} saved_guides={} curve_refresh_us={:.3} products={expected:?}",
            zoning.cells.saved_cells().len(),
            zoning.cells.saved_curve_sources().len(),
            samples[10]
        );
    }
}
