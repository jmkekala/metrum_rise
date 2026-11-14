// SPDX-License-Identifier: GPL-2.0-only

//! The road drawing holds' rule in Rust (`ROAD-112` round G2, docs/terrain.md T61 (2) and T65): an
//! install of road chunks made while the graded journal holds an edit the terrain renderer has not
//! installed keeps the chunks it replaces drawn until that edit installs, so the road and the
//! terrain cut under it change in one frame; a commit's preview stays drawn until the committed
//! generation's chunks are installed and no install is held. The GDScript keeps the Godot objects
//! (the held and hidden chunk instances, the swapped preview display) and asks this class when to
//! hold and when to release. Since T65 option (c) a preview frame's own lag needs no hold: the
//! published tiles' cut ground draws as the backstop (`graded_cut`).

use godot::prelude::*;

// The holds' state: the graded edit serial a held install waits for and the road mesh generation
// whose install releases a held preview, -1 when nothing is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Holds {
    serial: i64,
    preview: i64,
}

impl Default for Holds {
    fn default() -> Self {
        Self {
            serial: -1,
            preview: -1,
        }
    }
}

impl Holds {
    // An install: held for the journal's `journal` serial unless it replaces every chunk or the
    // renderer has installed it (`installed`); the hold covers every held install so far.
    fn install(&mut self, full_replace: bool, journal: i64, installed: i64) -> i64 {
        let hold = if !full_replace && journal > installed {
            journal
        } else {
            -1
        };
        self.serial = self.serial.max(hold);
        hold
    }

    // The held install swaps in once the renderer installed its edit, or now with `force`.
    fn release(&mut self, installed: i64, force: bool) -> bool {
        if self.serial < 0 || (!force && installed < self.serial) {
            return false;
        }
        self.serial = -1;
        true
    }

    // A held preview goes once its generation's chunks are installed and none is held.
    fn release_preview(&mut self, generation: i64) -> bool {
        if self.preview < 0 || generation < self.preview || self.serial >= 0 {
            return false;
        }
        self.preview = -1;
        true
    }
}

/// The road drawing holds (T61 (2)): when an install of road chunks or a commit's preview waits
/// for the graded terrain renderer, and when it goes. The chunk renderer and the road tool share
/// one; it keeps serials and generations only, the Godot objects stay theirs. Every call `O(1)`.
#[derive(GodotClass, Default)]
#[class(init, base = RefCounted)]
pub struct RoadDrawHolds {
    holds: Holds,
}

#[godot_api]
impl RoadDrawHolds {
    /// An install of road chunks: returns the graded edit serial it waits for (its replaced chunks
    /// stay drawn and its own hidden until then), -1 to draw now. `journal_serial` is the graded
    /// journal's serial at the install (-1 without a graded renderer), `installed_serial` the
    /// renderer's `installed_edit_serial`; a `full_replace` (another world) never waits.
    #[func]
    pub fn hold_install(
        &mut self,
        full_replace: bool,
        journal_serial: i64,
        installed_serial: i64,
    ) -> i64 {
        self.holds
            .install(full_replace, journal_serial, installed_serial)
    }

    /// Whether the held install swaps in now: the renderer installed the edits up to its serial
    /// (`installed_serial`; without a graded renderer pass `i64::MAX`), or `force`. True ends the
    /// hold; the caller frees the held chunks and shows the hidden ones.
    #[func]
    pub fn release_install(&mut self, installed_serial: i64, force: bool) -> bool {
        self.holds.release(installed_serial, force)
    }

    /// Whether an install is held: its chunks are not drawn yet.
    #[func]
    pub fn is_holding(&self) -> bool {
        self.holds.serial >= 0
    }

    /// The graded edit serial the held install waits for, -1 when none is held.
    #[func]
    pub fn held_serial(&self) -> i64 {
        self.holds.serial
    }

    /// An accepted commit's preview stays drawn until the chunks of `generation` are installed.
    #[func]
    pub fn hold_preview(&mut self, generation: i64) {
        self.holds.preview = generation;
    }

    /// Whether the held preview goes now: the installed chunks are of `chunks_generation`, at
    /// least its own, and no install is held. True ends the hold; the caller drops the preview.
    #[func]
    pub fn release_preview(&mut self, chunks_generation: i64) -> bool {
        self.holds.release_preview(chunks_generation)
    }

    /// The road mesh generation a held commit preview waits for, -1 when none is held.
    #[func]
    pub fn held_preview(&self) -> i64 {
        self.holds.preview
    }

    /// Drops the preview hold (its display was reset: a new commit's preview, a cancel).
    #[func]
    pub fn drop_preview(&mut self) {
        self.holds.preview = -1;
    }
}

#[cfg(test)]
mod tests {
    use super::Holds;

    // The chunk renderer's T61 (2) rule as `road_chunk_renderer.gd` applied it: an install waits
    // for a journal serial the renderer has not installed, a full replace never; the hold covers
    // every held install and releases once the renderer reaches it, or by force.
    #[test]
    fn an_install_holds_until_the_renderer_installs_its_edit() {
        let mut holds = Holds::default();
        assert_eq!(holds.install(false, 4, 4), -1, "installed: draws now");
        assert_eq!(holds.install(false, -1, 0), -1, "no graded renderer");
        assert_eq!(holds.install(true, 7, 4), -1, "a full replace");
        assert_eq!(holds.install(false, 6, 4), 6);
        assert_eq!(
            holds.install(false, 5, 4),
            5,
            "a later install waits for its own"
        );
        assert_eq!(holds.serial, 6, "the hold covers both");
        assert!(!holds.release(5, false));
        assert!(holds.release(6, false));
        assert!(!holds.release(9, false), "nothing held");
        holds.install(false, 8, 4);
        assert!(holds.release(4, true), "forced");
        assert_eq!(holds, Holds::default());
    }

    // The road tool's held commit preview: released once its generation's chunks are installed
    // and no install is held, never before.
    #[test]
    fn a_held_preview_goes_with_its_generations_chunks() {
        let mut holds = Holds::default();
        assert!(!holds.release_preview(10), "nothing held");
        holds.preview = 12;
        assert!(!holds.release_preview(11));
        holds.install(false, 3, 2);
        assert!(!holds.release_preview(12), "the chunks are held");
        holds.release(3, false);
        assert!(holds.release_preview(12));
        assert_eq!(holds, Holds::default());
    }
}
