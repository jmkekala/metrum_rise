// SPDX-License-Identifier: GPL-2.0-only

//! The stream benchmark's drawn probe (`RENDER-32-N`, `CODE-12`'s go/no-go measurement): for
//! every layer, how many frames a chunk that enters the layer's installed set waits until it is
//! drawn, and every in-view hole (a frame where an installed-set chunk inside the camera's
//! frustum is not drawn) with its length.
//!
//! Benchmark only: nothing in the game calls it. Each frame, after the frame drew
//! (`frame_post_draw`), the benchmark hands every layer's installed set as its owner reports it
//! (`probe_*` functions: one [`LANES`]-float record per chunk, its key, [`DRAWN`] / [`COVERED`] /
//! [`READY`] flags and its ground rectangle) to [`StreamDrawnProbe`], which keeps one
//! [`LayerTracker`] per layer. A chunk in a frame's set that was not in the previous frame's has
//! entered; it waits from that frame until the first frame it is drawn (0: drawn in the frame it
//! entered). Waits and holes are counted for the epoch (the benchmark's sample) the chunk entered
//! or the hole began in, so a chunk that enters in a motion's last frames and draws during the
//! settle after it still counts for that motion.
//!
//! Two clocks per chunk: drawn (the chunk itself draws: the pass bar) and visible (it draws, or
//! a stand-in draws over its area: a coarse vegetation patch over a fine one, fine stand-ins
//! over a coarse one, a graded tile drawn at another level). A waiting frame whose product is
//! [`READY`] (prepared, waiting only for its install) is counted apart: such a wait is the
//! install budget's, not the preparation's.
//!
//! Costs: a frame `O(chunks in the installed sets x frustum planes)`, on the benchmark's
//! instrumented runs only.

use godot::prelude::*;
use std::collections::HashMap;

type Map<K, V> = HashMap<K, V, foldhash::fast::FixedState>;

/// Floats per chunk record: key (three lanes), flags, ground rectangle (min x, min z, max x,
/// max z).
pub(crate) const LANES: usize = 8;
/// The chunk draws (the version its layer drew last, whatever its level).
pub(crate) const DRAWN: u32 = 1;
/// Something draws over the chunk's area while it does not (a stand-in, another level).
pub(crate) const COVERED: u32 = 2;
/// The chunk's product is prepared; it waits for its install only.
pub(crate) const READY: u32 = 4;

/// Appends one chunk record to `out`. `rect` is `[min x, min z, max x, max z]`.
pub(crate) fn push_chunk(out: &mut Vec<f32>, key: [i32; 3], flags: u32, rect: [f32; 4]) {
    out.extend([key[0] as f32, key[1] as f32, key[2] as f32, flags as f32]);
    out.extend(rect);
}

// A chunk of the installed set as last seen.
#[derive(Clone, Copy, Debug)]
struct Chunk {
    seen: u64,
    entered: u64,
    epoch: u32,
    waiting: bool,
    waiting_visible: bool,
    ready_frames: u32,
    // The current in-view runs not drawn and not visible (frames), and the epochs they began in.
    hole: u32,
    hole_epoch: u32,
    bare: u32,
    bare_epoch: u32,
}

/// One epoch's (sample's) counts for one layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EpochStats {
    /// Chunks that entered the installed set.
    pub(crate) entered: u32,
    /// Frames from entering to drawn, per chunk drawn.
    pub(crate) waits: Vec<u32>,
    /// Frames from entering to visible (drawn or covered), per chunk.
    pub(crate) waits_visible: Vec<u32>,
    /// Of each drawn chunk's wait, the frames its product was ready (the install budget's).
    pub(crate) ready_frames: Vec<u32>,
    /// Chunks that left the installed set before they drew.
    pub(crate) left_undrawn: u32,
    /// In-view runs of frames a chunk was not drawn, by length.
    pub(crate) holes: Vec<u32>,
    /// In-view runs of frames nothing drew over a chunk's area, by length.
    pub(crate) bare: Vec<u32>,
    /// Chunk frames in view not drawn.
    pub(crate) hole_frames: u32,
    /// The most chunks in view not drawn in one frame.
    pub(crate) holes_max: u32,
}

/// The entered-to-drawn and hole bookkeeping of one layer.
#[derive(Default)]
pub(crate) struct LayerTracker {
    chunks: Map<[i32; 3], Chunk>,
    // The first frame seen: its chunks are the baseline, not entries.
    started: bool,
    stats: Vec<EpochStats>,
}

impl LayerTracker {
    fn stats(&mut self, epoch: u32) -> &mut EpochStats {
        stat(&mut self.stats, epoch)
    }

    /// Takes frame `frame`'s installed set (`data`, [`LANES`] floats a chunk) in `epoch`, with the
    /// frustum `planes` (`[nx, ny, nz, d]`, outward: a point is outside when `n.p > d`) and the
    /// layer's height range `y`. `O(chunks x planes)`.
    pub(crate) fn observe(&mut self, frame: u64, epoch: u32, data: &[f32], planes: &[[f32; 4]], y: [f32; 2]) {
        let baseline = !self.started;
        self.started = true;
        let mut in_view_holes = 0;
        for record in data.chunks_exact(LANES) {
            let key = [record[0] as i32, record[1] as i32, record[2] as i32];
            let flags = record[3] as u32;
            let drawn = flags & DRAWN != 0;
            let visible = drawn || flags & COVERED != 0;
            let in_view = in_frustum(planes, [record[4], y[0], record[5]], [record[6], y[1], record[7]]);
            let previous = match self.chunks.get(&key).copied() {
                Some(chunk) if chunk.seen + 1 == frame => Some(chunk),
                // Not seen last frame (no frame was observed between): it left and enters again.
                Some(chunk) => {
                    leave(&mut self.stats, &chunk);
                    None
                }
                None => None,
            };
            let mut chunk = match previous {
                Some(chunk) => chunk,
                None => {
                    let fresh = Chunk {
                        seen: frame,
                        entered: frame,
                        epoch,
                        waiting: !baseline && !drawn,
                        waiting_visible: !baseline && !visible,
                        ready_frames: 0,
                        hole: 0,
                        hole_epoch: epoch,
                        bare: 0,
                        bare_epoch: epoch,
                    };
                    if !baseline {
                        let stats = self.stats(epoch);
                        stats.entered += 1;
                        if drawn {
                            stats.waits.push(0);
                            stats.ready_frames.push(0);
                        }
                        if visible {
                            stats.waits_visible.push(0);
                        }
                    }
                    fresh
                }
            };
            chunk.seen = frame;
            let waited = (frame - chunk.entered) as u32;
            if chunk.waiting {
                if drawn {
                    chunk.waiting = false;
                    let ready = chunk.ready_frames;
                    let stats = self.stats(chunk.epoch);
                    stats.waits.push(waited);
                    stats.ready_frames.push(ready);
                } else if flags & READY != 0 {
                    chunk.ready_frames += 1;
                }
            }
            if chunk.waiting_visible && visible {
                chunk.waiting_visible = false;
                self.stats(chunk.epoch).waits_visible.push(waited);
            }
            let (hole, bare) = (in_view && !drawn, in_view && !visible);
            Self::run(&mut chunk.hole, &mut chunk.hole_epoch, hole, epoch, |length, at| push_run(&mut self.stats, at, length, false));
            Self::run(&mut chunk.bare, &mut chunk.bare_epoch, bare, epoch, |length, at| push_run(&mut self.stats, at, length, true));
            if hole {
                in_view_holes += 1;
                self.stats(epoch).hole_frames += 1;
            }
            self.chunks.insert(key, chunk);
        }
        let stats = self.stats(epoch);
        stats.holes_max = stats.holes_max.max(in_view_holes);
        // Chunks that left the set: an unfinished wait is dropped and counted, an open run ends.
        let stats = &mut self.stats;
        self.chunks.retain(|_, chunk| {
            if chunk.seen == frame {
                return true;
            }
            leave(stats, chunk);
            false
        });
    }

    // Advances a run of frames: extends it while `on`, ends it (reported through `end`) when not.
    fn run(length: &mut u32, began: &mut u32, on: bool, epoch: u32, mut end: impl FnMut(u32, u32)) {
        if on {
            if *length == 0 {
                *began = epoch;
            }
            *length += 1;
        } else if *length > 0 {
            end(*length, *began);
            *length = 0;
        }
    }

    /// The counts by epoch, with the waits and runs still open counted as they stand (`open`:
    /// chunks still waiting, with their waits so far).
    pub(crate) fn report(&self, frame: u64) -> (Vec<EpochStats>, Vec<Vec<u32>>) {
        let mut stats = self.stats.clone();
        let mut open = vec![Vec::new(); stats.len()];
        for chunk in self.chunks.values() {
            if chunk.waiting {
                open[chunk.epoch as usize].push((frame - chunk.entered) as u32);
            }
            if chunk.hole > 0 {
                stats[chunk.hole_epoch as usize].holes.push(chunk.hole);
            }
            if chunk.bare > 0 {
                stats[chunk.bare_epoch as usize].bare.push(chunk.bare);
            }
        }
        (stats, open)
    }
}

// A chunk leaves the set: an unfinished wait is counted as left undrawn, open runs end.
fn leave(stats: &mut Vec<EpochStats>, chunk: &Chunk) {
    if chunk.waiting {
        stat(stats, chunk.epoch).left_undrawn += 1;
    }
    if chunk.hole > 0 {
        stat(stats, chunk.hole_epoch).holes.push(chunk.hole);
    }
    if chunk.bare > 0 {
        stat(stats, chunk.bare_epoch).bare.push(chunk.bare);
    }
}

// `epoch`'s counts, grown on first use.
fn stat(stats: &mut Vec<EpochStats>, epoch: u32) -> &mut EpochStats {
    let index = epoch as usize;
    if stats.len() <= index {
        stats.resize(index + 1, EpochStats::default());
    }
    &mut stats[index]
}

// Records a finished run of `length` frames that began in `epoch`.
fn push_run(stats: &mut Vec<EpochStats>, epoch: u32, length: u32, bare: bool) {
    let stats = stat(stats, epoch);
    if bare {
        stats.bare.push(length);
    } else {
        stats.holes.push(length);
    }
}

/// Whether the box `low`..`high` meets the frustum `planes` (outward normals): it is outside when
/// its corner farthest inward is still over a plane, as `terrain_graded_tiles.gd` tests a patch.
/// No planes: in view.
pub(crate) fn in_frustum(planes: &[[f32; 4]], low: [f32; 3], high: [f32; 3]) -> bool {
    planes.iter().all(|plane| {
        let corner = [0, 1, 2].map(|axis| if plane[axis] < 0.0 { high[axis] } else { low[axis] });
        let over = plane[0] * corner[0] + plane[1] * corner[1] + plane[2] * corner[2] > plane[3];
        !over
    })
}

/// The benchmark's drawn probe: one [`LayerTracker`] per layer name, frames counted by
/// [`Self::begin_frame`], epochs named by [`Self::set_epoch`].
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct StreamDrawnProbe {
    layers: Vec<(GString, LayerTracker)>,
    epochs: Vec<GString>,
    epoch: u32,
    frame: u64,
    planes: Vec<[f32; 4]>,
}

#[godot_api]
impl StreamDrawnProbe {
    /// Starts a frame: the camera's frustum as `Camera3D.get_frustum()` returns it.
    #[func]
    pub fn begin_frame(&mut self, planes: Array<Plane>) {
        self.frame += 1;
        self.planes.clear();
        self.planes.extend(planes.iter_shared().map(|plane| [plane.normal.x, plane.normal.y, plane.normal.z, plane.d]));
    }

    /// Counts what follows for the epoch `label` (a sample's label; empty: between samples).
    #[func]
    pub fn set_epoch(&mut self, label: GString) {
        self.epoch = match self.epochs.iter().position(|known| *known == label) {
            Some(index) => index as u32,
            None => {
                self.epochs.push(label);
                (self.epochs.len() - 1) as u32
            }
        };
    }

    /// Takes layer `name`'s installed set this frame (`probe_*`'s records) with its height range
    /// `y_min`..`y_max`. `O(chunks x planes)`.
    #[func]
    pub fn observe(&mut self, name: GString, data: PackedFloat32Array, y_min: f64, y_max: f64) {
        let index = match self.layers.iter().position(|(known, _)| *known == name) {
            Some(index) => index,
            None => {
                self.layers.push((name, LayerTracker::default()));
                self.layers.len() - 1
            }
        };
        let (frame, epoch) = (self.frame, self.epoch);
        self.layers[index].1.observe(frame, epoch, data.as_slice(), &self.planes, [y_min as f32, y_max as f32]);
    }

    /// The counts: per epoch label, per layer `entered`, `waits`, `waits_visible`,
    /// `ready_frames`, `open` (still waiting: their waits so far), `left_undrawn`, `holes`,
    /// `bare`, `hole_frames`, `holes_max`; `frames` the frames observed.
    #[func]
    pub fn report(&self) -> VarDictionary {
        let mut out = VarDictionary::new();
        let packed = |values: &[u32]| PackedInt32Array::from_iter(values.iter().map(|&value| value as i32));
        for (name, layer) in &self.layers {
            let (stats, open) = layer.report(self.frame);
            for (epoch, (stats, open)) in stats.iter().zip(&open).enumerate() {
                let label = self.epochs.get(epoch).cloned().unwrap_or_default();
                let mut by_layer: VarDictionary = out.get(label.clone()).map_or_else(VarDictionary::new, |value| value.to());
                let mut entry = VarDictionary::new();
                entry.set("entered", stats.entered);
                entry.set("waits", packed(&stats.waits));
                entry.set("waits_visible", packed(&stats.waits_visible));
                entry.set("ready_frames", packed(&stats.ready_frames));
                entry.set("open", packed(open));
                entry.set("left_undrawn", stats.left_undrawn);
                entry.set("holes", packed(&stats.holes));
                entry.set("bare", packed(&stats.bare));
                entry.set("hole_frames", stats.hole_frames);
                entry.set("holes_max", stats.holes_max);
                by_layer.set(name.clone(), entry);
                out.set(label, by_layer);
            }
        }
        out.set("frames", self.frame as i64);
        out
    }
}

#[cfg(test)]
mod tests;
