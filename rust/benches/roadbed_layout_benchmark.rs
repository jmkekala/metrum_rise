// SPDX-License-Identifier: GPL-2.0-only

//! Isolated junction-layout benchmark (`ROAD-68-C`): one T, four-way and six-way node each, with
//! straight two-lane arms carrying sidewalks and crosswalks, 6 m knots and 50 m of reach.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use metrum_rise::simulation::network::roadbed::{
    ArmBand, ArmInput, ArmKey, ArmKnot, ArmSide, BandKind, EdgeEnd, JunctionLayout, Level, RoadVec2,
};

const HALF_CARRIAGEWAY_M: f64 = 3.5;
const REACH_M: f64 = 50.0;
const KNOT_SPACING_M: f64 = 6.0;

struct Fixture {
    knots: Vec<Vec<ArmKnot>>,
    bands: [ArmBand; 3],
}

impl Fixture {
    fn new(node: RoadVec2, angles_deg: &[f64]) -> Self {
        let knots = angles_deg
            .iter()
            .map(|angle| {
                let direction = RoadVec2::from_angle(angle.to_radians());
                let count = (REACH_M / KNOT_SPACING_M).ceil() as usize;
                (0..=count)
                    .map(|index| {
                        let station_m = (index as f64 * KNOT_SPACING_M).min(REACH_M);
                        ArmKnot {
                            position: if index == 0 {
                                node
                            } else {
                                node + direction * station_m
                            },
                            lateral: direction.perp(),
                            station_m,
                        }
                    })
                    .collect()
            })
            .collect();
        let band = |kind, level, outer_m| ArmBand {
            kind,
            level,
            outer_m,
        };
        Self {
            knots,
            bands: [
                band(BandKind::Carriageway, Level::Ground, HALF_CARRIAGEWAY_M),
                band(
                    BandKind::CurbOrShoulder,
                    Level::Raised,
                    HALF_CARRIAGEWAY_M + 0.15,
                ),
                band(BandKind::Sidewalk, Level::Raised, HALF_CARRIAGEWAY_M + 1.5),
            ],
        }
    }

    fn arms(&self) -> Vec<ArmInput<'_>> {
        let side = ArmSide {
            bands: &self.bands,
            core_bands: 1,
        };
        self.knots
            .iter()
            .enumerate()
            .map(|(edge, knots)| ArmInput {
                key: ArmKey {
                    edge,
                    end: EdgeEnd::Start,
                },
                knots,
                left: side,
                right: side,
                max_station_m: REACH_M,
                crossable: true,
            })
            .collect()
    }
}

fn layout_benchmark(c: &mut Criterion) {
    let node = RoadVec2::new(4321.5, -2345.25);
    let cases: [(&str, &[f64]); 3] = [
        ("t", &[13.0, 193.0, 283.0]),
        ("four_way", &[13.0, 103.0, 193.0, 283.0]),
        ("six_way", &[7.0, 67.0, 127.0, 187.0, 247.0, 307.0]),
    ];
    let mut group = c.benchmark_group("layout");
    for (name, angles) in cases {
        let fixture = Fixture::new(node, angles);
        let arms = fixture.arms();
        JunctionLayout::build(node, &arms).expect("benchmark junction lays out");
        group.bench_function(BenchmarkId::from_parameter(name), |bencher| {
            bencher.iter(|| JunctionLayout::build(black_box(node), black_box(&arms)))
        });
    }
    group.finish();
}

criterion_group!(benches, layout_benchmark);
criterion_main!(benches);
