// SPDX-License-Identifier: GPL-2.0-only

//! The zoning ground masks' resident chunks on a ring (`RENDER-32-P`, `docs/terrain.md`
//! "Streaming core"): while the zoning overlay shows, every zoning chunk whose square reaches
//! within [`ZONING_RING_M`] of the eye on the ground is resident, with [`ZONING_HYSTERESIS_M`]
//! before it leaves, instead of the box of the four screen corners' rays that
//! `zoning_cells_overlay.gd` derived every frame. The set no longer turns with the camera: yaw
//! never enters it, and a frame whose eye moved inside the hysteresis changes nothing.
//!
//! A [`RingSet`] over the zoning chunk grid keeps each chunk's band (inside the ring, in the
//! hysteresis band, out); a chunk enters when it comes inside and leaves when it goes out, so a
//! move re-evaluates the chunks near the two thresholds only: `O(ring boundary chunks)` a move,
//! nothing on a still frame, `O(new + old set)` for a first point or a jump. The resident list,
//! nearest first, is rebuilt only when the set changed, `O(n log n)` for its `n` chunks. Nothing
//! allocates once the storage has grown.

use crate::nodes::sim::render::stream::rings::{Cell, Lattice, RingBand, RingSet};
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// The ring's radius, metres: the corner rays' reach (`VIEW_DISTANCE_M`, 2 km) plus the one-chunk
/// pad they took is covered by measuring to the chunk's nearest point.
pub(crate) const ZONING_RING_M: f64 = 2000.0;
/// How much farther a resident chunk may be before it leaves, metres.
pub(crate) const ZONING_HYSTERESIS_M: f64 = 64.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Band {
    #[default]
    Out,
    Hysteresis,
    Inside,
}

/// The resident zoning chunks.
pub(crate) struct ZoningRing {
    ring: RingSet<Band>,
    span: f64,
    // The chunk index range of the world, inclusive.
    first: [i32; 2],
    last: [i32; 2],
    resident: Map<Cell, ()>,
    // The resident chunks nearest first, when `sorted` (reused storage).
    list: Vec<(f64, Cell)>,
    sorted: bool,
    revision: u64,
}

impl ZoningRing {
    /// A ring over `span`-metre chunks (chunk `[0, 0]` starts at the world's centre) of a world
    /// of `world` metres about its centre.
    pub(crate) fn new(span: f64, world: [f64; 2]) -> Self {
        let index = |metres: f64| (metres / span).floor() as i32;
        Self {
            ring: RingSet::new(Lattice {
                origin: [0.0, 0.0],
                cell: span,
            }),
            span,
            first: [index(-world[0] * 0.5), index(-world[1] * 0.5)],
            last: [index(world[0] * 0.5), index(world[1] * 0.5)],
            resident: Map::default(),
            list: Vec::new(),
            sorted: true,
            revision: 0,
        }
    }

    /// Moves the ring to the eye `[x, z]`; true when the resident set changed.
    pub(crate) fn update(&mut self, eye: [f64; 2]) -> bool {
        // A cell's nearest point lies up to half its diagonal nearer than its centre.
        let reach = self.span * std::f64::consts::FRAC_1_SQRT_2;
        let bands = [
            RingBand {
                radius: ZONING_RING_M,
                reach,
            },
            RingBand {
                radius: ZONING_RING_M + ZONING_HYSTERESIS_M,
                reach,
            },
        ];
        let (span, first, last) = (self.span, self.first, self.last);
        let resident = &mut self.resident;
        let mut changed = false;
        self.ring.update(
            eye,
            &bands,
            false,
            |cell| band(cell, eye, span, first, last),
            |cell, _, new| match new {
                Band::Inside => changed |= resident.insert(cell, ()).is_none(),
                Band::Out => changed |= resident.remove(&cell).is_some(),
                Band::Hysteresis => {}
            },
        );
        if changed {
            self.sorted = false;
            self.revision += 1;
        }
        changed
    }

    /// Advances whenever the resident set changes.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The resident chunks, nearest first from the last eye (then by key).
    pub(crate) fn resident(&mut self) -> impl Iterator<Item = Cell> + '_ {
        if !self.sorted {
            let eye = self.ring.point().unwrap_or_default();
            let span = self.span;
            self.list.clear();
            self.list.extend(self.resident.keys().map(|&cell| {
                let centre = |at: i32, eye: f64| (f64::from(at) + 0.5) * span - eye;
                let (dx, dz) = (centre(cell[0], eye[0]), centre(cell[1], eye[1]));
                (dx * dx + dz * dz, cell)
            }));
            self.list
                .sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            self.sorted = true;
        }
        self.list.iter().map(|&(_, cell)| cell)
    }

    /// The chunks the last update and refreshes visited (tests: a still frame visits none).
    #[cfg(test)]
    pub(crate) fn visited(&mut self) -> usize {
        let visited = self.ring.visited();
        self.ring.reset_visited();
        visited
    }
}

// A chunk's band at `eye`: by the ground distance to its nearest point; out beyond the world.
fn band(cell: Cell, eye: [f64; 2], span: f64, first: [i32; 2], last: [i32; 2]) -> Band {
    if (0..2).any(|axis| cell[axis] < first[axis] || cell[axis] > last[axis]) {
        return Band::Out;
    }
    let gap = |at: i32, eye: f64| {
        let low = f64::from(at) * span;
        (low - eye).max(eye - (low + span)).max(0.0)
    };
    let (dx, dz) = (gap(cell[0], eye[0]), gap(cell[1], eye[1]));
    let distance = (dx * dx + dz * dz).sqrt();
    if distance <= ZONING_RING_M {
        Band::Inside
    } else if distance <= ZONING_RING_M + ZONING_HYSTERESIS_M {
        Band::Hysteresis
    } else {
        Band::Out
    }
}

#[cfg(test)]
mod tests;
