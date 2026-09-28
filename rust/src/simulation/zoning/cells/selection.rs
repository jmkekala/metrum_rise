// SPDX-License-Identifier: GPL-2.0-only

//! Shared whole-cell gesture selection and atomic profile/erase deltas.

use super::CellCurveSource;
use super::geometry::{segment_intersects_cell, valid_point};
use super::{CellBounds, CellKey, CellStore};
use glam::DVec2;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

/// Selection geometry is independent of the selected profile or Erase operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CellSelectionShape {
    /// Every cell touched by the continuous pointer path.
    Cell,
    /// Cell centres inside a rectangle in the initial clicked cell's frame.
    Marquee,
    /// Four-neighbor cells with the clicked profile and grid identity.
    Fill,
    /// Cell centres inside a circular brush swept along the pointer path.
    Brush {
        /// World radius in metres, independent of camera zoom.
        radius_m: f64,
    },
}

/// One non-mutating Rust-authored selection, shared by preview and commit.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CellSelection {
    /// Authority epoch at selection time; stale gestures are rejected before any write.
    pub(crate) revision: u64,
    /// Deterministically ordered unique addresses to preview and edit.
    pub(crate) cells: Vec<CellKey>,
}

/// Local inverse information for a single committed gesture, including erase.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CellEdit {
    // The last erased cell may release a group's generated cache before undo.
    sources: Vec<Arc<CellCurveSource>>,
    /// Only changed cells and their previous paint; no world snapshot is captured.
    pub(crate) previous: Vec<(CellKey, u16)>,
    /// New profile assigned to the changed cells, with zero meaning Erase.
    pub(crate) profile: u16,
    /// Epoch after the edit, used to reject undo across a conflicting state change.
    pub(crate) revision: u64,
}

impl CellStore {
    /// Computes a gesture from all sampled pointer positions; long segments are swept exactly.
    pub(crate) fn select(&self, shape: CellSelectionShape, path: &[DVec2]) -> CellSelection {
        let mut result = CellSelection {
            revision: self.revision(),
            cells: Vec::new(),
        };
        let Some(&start) = path.first() else {
            return result;
        };
        if !path.iter().copied().all(valid_point) {
            return result;
        }
        if shape == CellSelectionShape::Fill {
            if let Some(seed) = self.pick(start) {
                self.select_fill(seed, &mut result.cells);
            }
            // Fixed neighbor order already gives deterministic, unique O(F) traversal.
            return result;
        } else if shape == CellSelectionShape::Cell && path.iter().all(|&point| point == start) {
            result.cells.extend(self.pick(start));
            return result;
        } else if shape == CellSelectionShape::Marquee {
            if let Some(seed) = self.pick(start)
                && let Some(frame) = self.frame(seed.grid)
            {
                let a = frame.local(start);
                let b = frame.local(path[path.len() - 1]);
                let min = a.min(b);
                let max = a.max(b);
                let bounds = CellBounds::from_points([
                    frame.world(min.x, min.y),
                    frame.world(max.x, min.y),
                    frame.world(max.x, max.y),
                    frame.world(min.x, max.y),
                ]);
                self.visit_in_bounds(bounds, |key| {
                    if let Some(candidate_frame) = self.frame(key.grid) {
                        let centre =
                            candidate_frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5);
                        let local = frame.local(centre);
                        if local.cmpge(min).all() && local.cmple(max).all() {
                            result.cells.push(key);
                        }
                    }
                });
            }
        } else {
            let radius = match shape {
                CellSelectionShape::Brush { radius_m }
                    if radius_m.is_finite() && radius_m >= 0.0 =>
                {
                    radius_m
                }
                CellSelectionShape::Cell => 0.0,
                _ => return result,
            };
            for index in 0..path.len() {
                let a = path[index.saturating_sub(1)];
                let b = path[index];
                let bounds = CellBounds::from_points([a, b]).expanded(radius);
                self.visit_in_bounds(bounds, |key| {
                    let Some(frame) = self.frame(key.grid) else {
                        return;
                    };
                    let selected = if shape == CellSelectionShape::Cell {
                        segment_intersects_cell(&frame.corners(key.x, key.y), a, b)
                    } else {
                        let centre = frame.world(f64::from(key.x) + 0.5, f64::from(key.y) + 0.5);
                        let segment = b - a;
                        let t = if segment.length_squared() > 0.0 {
                            ((centre - a).dot(segment) / segment.length_squared()).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        centre.distance_squared(a + t * segment) <= radius * radius
                    };
                    if selected {
                        result.cells.push(key);
                    }
                });
            }
        }
        result.cells.sort_unstable();
        result.cells.dedup();
        result
    }

    fn select_fill(&self, seed: CellKey, result: &mut Vec<CellKey>) {
        let Some(profile) = self.profile(seed) else {
            return;
        };
        fill_cells(seed, |key| self.profile(key) == Some(profile), result);
    }

    /// Commits validated selection atomically. Registry and land-reservation checks belong to
    /// `ZoningSystem`; this store enforces stable cell identity and returns a local inverse.
    pub(crate) fn paint(&mut self, selection: &CellSelection, profile: u16) -> Option<CellEdit> {
        if selection.revision != self.revision()
            || !selection
                .cells
                .iter()
                .all(|&key| self.profile(key).is_some())
        {
            return None;
        }
        let mut previous = Vec::with_capacity(selection.cells.len());
        for &key in &selection.cells {
            if let Some(old) = self.profile(key)
                && old != profile
            {
                previous.push((key, old));
                self.set_profile(key, profile);
            }
        }
        if !previous.is_empty() {
            self.bump_revision();
        }
        Some(CellEdit {
            sources: self.capture_curve_sources(&selection.cells),
            previous,
            profile,
            revision: self.revision(),
        })
    }

    /// Applies the inverse only while all post-edit identities and values still match.
    pub(crate) fn undo_paint(&mut self, edit: &CellEdit) -> bool {
        if edit.revision != self.revision() {
            return false;
        }
        self.restore_paint_values(edit)
    }

    /// Restores a historical gesture after its caller validates current reservations. Local
    /// Paint must still match. Erased addresses evicted by cache regeneration can be restored
    /// if their exact footprints remain unreserved; generated empty competitors are replaced.
    pub(crate) fn restore_paint_values(&mut self, edit: &CellEdit) -> bool {
        let mut missing = Vec::new();
        for &(key, profile) in &edit.previous {
            if self.profile(key) == Some(edit.profile) {
                continue;
            }
            if edit.profile != 0 || profile == 0 || self.profile(key).is_some() {
                return false;
            }
            let Some(frame) = self.frame(key.grid) else {
                return false;
            };
            let corners = frame.corners(key.x, key.y);
            if self.overlaps_reserved(&corners) {
                return false;
            }
            missing.push((key, corners));
        }
        // All authority checks precede every mutation, including removal of cache-only cells.
        let mut replace = Vec::new();
        for (key, corners) in missing {
            replace.clear();
            self.visit_in_bounds(CellBounds::from_points(corners), |other| {
                if self.frame(other.grid).is_some_and(|frame| {
                    super::geometry::interiors_overlap(&corners, &frame.corners(other.x, other.y))
                }) {
                    replace.push(other);
                }
            });
            for &other in &replace {
                self.remove_unreserved(other);
            }
            self.insert(key);
        }
        for &(key, profile) in &edit.previous {
            self.set_profile(key, profile);
        }
        for source in &edit.sources {
            self.publish_curve_source(Arc::clone(source));
        }
        if !edit.previous.is_empty() {
            self.bump_revision();
        }
        true
    }
}

/// Shared deterministic four-neighbor traversal; materializing queries resolve a neighbor
/// before deciding whether it belongs to the clicked grid/profile component.
pub(super) fn fill_cells(
    seed: CellKey,
    mut matches: impl FnMut(CellKey) -> bool,
    result: &mut Vec<CellKey>,
) {
    let mut seen = HashSet::from([seed]);
    let mut queue = VecDeque::from([seed]);
    while let Some(cell) = queue.pop_front() {
        result.push(cell);
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let (Some(x), Some(y)) = (cell.x.checked_add(dx), cell.y.checked_add(dy)) else {
                continue;
            };
            let neighbor = CellKey {
                grid: cell.grid,
                x,
                y,
            };
            if seen.insert(neighbor) && matches(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
}
