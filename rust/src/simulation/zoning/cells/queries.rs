// SPDX-License-Identifier: GPL-2.0-only

//! Gesture materialization independent of previously viewed or generated neighborhoods.

use super::{CellBounds, CellSelection, CellSelectionShape, CellStore};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::graph::RegionGraph;
use glam::DVec2;

impl CellStore {
    /// Generates only gesture-relevant chunks before selecting. Fill follows its component
    /// into uncached chunks; storage and previous camera coverage never stop the traversal.
    /// Work is local generation per visited chunk plus the ordinary selector's work.
    pub(crate) fn select_generated(
        &mut self,
        graph: &RegionGraph,
        config: &WorldConfig,
        shape: CellSelectionShape,
        path: &[DVec2],
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> CellSelection {
        let empty = |store: &Self| CellSelection {
            revision: store.revision(),
            cells: Vec::new(),
        };
        let Some(&start) = path.first() else {
            return empty(self);
        };
        if !path.iter().copied().all(super::geometry::valid_point) {
            return empty(self);
        }
        let radius = match shape {
            CellSelectionShape::Brush { radius_m } if radius_m.is_finite() && radius_m >= 0.0 => {
                radius_m
            }
            CellSelectionShape::Brush { .. } => return empty(self),
            _ => 0.0,
        };
        let half_world = DVec2::new(f64::from(config.width_m), f64::from(config.height_m)) * 0.5;
        let materialize = |store: &mut Self, bounds: CellBounds| {
            let bounds = CellBounds {
                min: bounds.min.max(-half_world),
                max: bounds.max.min(half_world),
            };
            if !bounds.is_valid() {
                return;
            }
            let size = f64::from(RegionGraph::CHUNK_SIZE);
            let min = (bounds.min / size).floor();
            let max = (bounds.max / size).floor();
            for x in min.x as i32..=max.x as i32 {
                for y in min.y as i32..=max.y as i32 {
                    if let Some(dirty) = store.chunk_generation_bounds((x, y)) {
                        store.generate_in_bounds(graph, config, dirty, &blocked);
                        store.complete_generated_chunk((x, y), dirty);
                    }
                }
            }
        };
        materialize(
            self,
            CellBounds {
                min: start,
                max: start,
            },
        );
        if shape == CellSelectionShape::Fill {
            let Some(seed) = self.pick(start) else {
                return empty(self);
            };
            let frame = self.frame(seed.grid).expect("picked cell frame");
            let profile = self.profile(seed);
            let mut cells = Vec::new();
            super::selection::fill_cells(
                seed,
                |key| {
                    // The center chooses a generation chunk; that chunk publishes every full
                    // square touching it, including squares crossing its boundary.
                    let center = frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5);
                    materialize(
                        self,
                        CellBounds {
                            min: center,
                            max: center,
                        },
                    );
                    self.profile(key) == profile
                },
                &mut cells,
            );
            return CellSelection {
                revision: self.revision(),
                cells,
            };
        }
        if shape == CellSelectionShape::Marquee {
            let Some(seed) = self.pick(start) else {
                return empty(self);
            };
            let frame = self.frame(seed.grid).expect("picked cell frame");
            let a = frame.local(start);
            let b = frame.local(path[path.len() - 1]);
            materialize(
                self,
                CellBounds::from_points([
                    frame.world(a.x, a.y),
                    frame.world(a.x, b.y),
                    frame.world(b.x, a.y),
                    frame.world(b.x, b.y),
                ]),
            );
        } else {
            for index in 0..path.len() {
                materialize(
                    self,
                    CellBounds::from_points([path[index.saturating_sub(1)], path[index]])
                        .expanded(radius),
                );
            }
        }
        self.select(shape, path)
    }
}
