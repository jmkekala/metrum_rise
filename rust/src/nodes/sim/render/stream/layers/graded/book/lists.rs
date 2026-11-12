// SPDX-License-Identifier: GPL-2.0-only

//! The graded patch lists' requests and arrivals in Rust (`RENDER-32-K` part 6), as
//! `terrain_graded_tiles.gd`'s `_request_patch_lists`, `_take_list` and `_failed` did them, in the
//! same order (the traces pin it): the patches whose list may need a request (a patch that
//! changed, or whose list arrived, failed or was not accepted), the lists in flight by token, an
//! arriving list counted into the desired set, and a failed request asked again or, refused by
//! contract, recorded with its token and not asked again at it. The lists and refusals
//! themselves are the state's (`Graded::lists`, `lists_refused`).
//!
//! Bounds: a frame's requests `O(patches wanted, at most 32 asked)`; an arrival `O(its tiles)`;
//! a failure `O(1)`.

use super::super::state::PatchKey;
use super::results::{Host, Run};
use super::*;

/// Patch list requests per frame: Rust holds at most 32 in flight.
const LIST_REQUESTS_PER_FRAME: usize = 32;

/// The list requests' state in the book.
#[derive(Default)]
pub(super) struct Lists {
    // Patch key -> the token of its list in flight.
    requested: OrderedMap<PatchKey, i64>,
    // The patches whose list may need a request, in the order they came.
    wanted: OrderedMap<PatchKey, ()>,
    // This frame's lists taken and those equal to the patch's last (the perf channel's).
    frame: [i64; 2],
    // Requests refused by contract (lists and tiles) since the last read.
    failures: i64,
    keys: Vec<PatchKey>,
}

impl Lists {
    pub(super) fn clear(&mut self) {
        self.requested.clear();
        self.wanted.clear();
    }

    pub(super) fn reset_frame(&mut self) {
        self.frame = [0; 2];
    }

    // `_patch_changed`'s part: a resident patch's list may need a request; one that left none.
    pub(super) fn patch_changed(&mut self, key: PatchKey, resident: bool) {
        if resident {
            self.wanted.insert(key, ());
        } else {
            self.wanted.remove(&key);
        }
    }
}

// A dictionary's value under `key`, its type's default when absent.
fn value<T: FromGodot + Default>(dict: &VarDictionary, key: &str) -> T {
    dict.get(key)
        .map_or_else(T::default, |value| value.to::<T>())
}

impl<H: Host> Run<H> {
    // `_take_list`: patch `key`'s list arrived; recorded, its tiles counted into the desired set,
    // and asked again if baked for an older generation.
    pub(super) fn take_list(&mut self, list: &VarDictionary) {
        let key = (
            value::<i64>(list, "patch_x") as i32,
            value::<i64>(list, "patch_z") as i32,
        );
        let token = value::<i64>(list, "token");
        let flat = value::<PackedInt32Array>(list, "tiles");
        let tiles: Vec<Tile> = flat
            .as_slice()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|tile| Tile {
                x: tile[0],
                z: tile[1],
            })
            .collect();
        {
            let mut book = self.host.book();
            book.list_requests.requested.remove(&key);
            book.list_requests.frame[0] += 1;
            if book
                .state
                .lists
                .get(&key)
                .is_some_and(|(_, previous)| *previous == tiles)
            {
                book.list_requests.frame[1] += 1;
            }
            // The cells show the installed tiles of the patch's rectangle, not its list: a list
            // that empties or fills only starts or stops the set's drawing (the book's draw
            // state); listed tiles install or evict on their own. A patch without a set builds
            // one.
            book.list_cells(Vector2i::new(key.0, key.1), true);
            // Counted into the desired set; the parked tiles of its blocks released once their
            // lists are in.
            book.state.take_list(key, token, tiles);
        }
        self.effects();
        // A list baked for an older generation is asked again.
        self.host.book().list_requests.wanted.insert(key, ());
    }

    // `_failed`: a transient failure is asked again; one Rust refuses by contract is recorded
    // with its token, counted and logged once, and not asked again until its token changes (an
    // edit, a new patch generation).
    pub(super) fn failed(&mut self, failure: &VarDictionary) {
        let keys = value::<PackedInt64Array>(failure, "keys");
        let keys = keys.as_slice();
        let token = value::<i64>(failure, "token");
        let retry = value::<bool>(failure, "retry");
        let kind = value::<GString>(failure, "kind");
        if kind == GString::from("patch") {
            let key = (keys[0] as i32, keys[1] as i32);
            let mut book = self.host.book();
            if book.list_requests.requested.get(&key) == Some(&token) {
                book.list_requests.requested.remove(&key);
            }
            if !retry {
                book.list_cells(Vector2i::new(key.0, key.1), false);
                book.state.list_refused(key, token);
                drop(book);
                self.effects();
                book = self.host.book();
            }
            book.list_requests.wanted.insert(key, ());
        } else {
            let tile = Vector2i::new(keys[0] as i32, keys[1] as i32);
            let mut book = self.host.book();
            // The book drops the request; a refusal by contract is recorded and leaves the edit
            // batch: the tile stays as it was, and its patches' sets draw without it.
            book.failed_tile(tile, keys[2], keys[3], token, retry);
            if !retry {
                book.unstage(tile_of(tile));
                book.tile_refused(tile);
            }
        }
        if !retry {
            self.host.book().list_requests.failures += 1;
            crate::debug_log!(
                "terrain",
                "graded {kind} {keys:?} token {token} refused: {}",
                value::<GString>(failure, "reason")
            );
        }
    }
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// The lists to request this frame (`_request_patch_lists`), flat `(x, z, generation)`: the
    /// wanted resident patches whose list is not of their generation, not in flight and not
    /// refused at it, at most 32 (the rest stay wanted); each is marked in flight. O(patches
    /// taken from the wanted).
    #[func]
    pub fn lists_to_request(&mut self) -> PackedInt64Array {
        let mut flat = Vec::new();
        let mut keys = std::mem::take(&mut self.list_requests.keys);
        keys.clear();
        keys.extend(self.list_requests.wanted.keys().copied());
        for &key in &keys {
            if flat.len() >= LIST_REQUESTS_PER_FRAME * 3 {
                break;
            }
            self.list_requests.wanted.remove(&key);
            let Some(patch) = self.state.resident.get(&key) else {
                continue;
            };
            let generation = patch.generation;
            if self
                .state
                .lists
                .get(&key)
                .is_some_and(|(token, _)| *token == generation)
            {
                continue;
            }
            if self.list_requests.requested.get(&key) == Some(&generation)
                || self.state.lists_refused.get(&key) == Some(&generation)
            {
                continue;
            }
            self.list_requests.requested.insert(key, generation);
            flat.extend([i64::from(key.0), i64::from(key.1), generation]);
        }
        keys.clear();
        self.list_requests.keys = keys;
        PackedInt64Array::from(flat.as_slice())
    }

    /// The simulation's answer to `lists_to_request`'s `flat`: a request it did not accept (its
    /// in-flight bound) is wanted again.
    #[func]
    pub fn lists_accepted(&mut self, flat: PackedInt64Array, accepted: PackedByteArray) {
        let flat = flat.as_slice();
        for (index, &answer) in accepted.as_slice().iter().enumerate() {
            if answer == 0 {
                let key = (flat[index * 3] as i32, flat[index * 3 + 1] as i32);
                self.list_requests.requested.remove(&key);
                self.list_requests.wanted.insert(key, ());
            }
        }
    }

    /// Whether a patch's list may need a request.
    #[func]
    pub fn has_lists_wanted(&self) -> bool {
        self.list_requests.wanted.len() > 0
    }

    /// Whether lists are in flight or may need a request.
    #[func]
    pub fn lists_pending(&self) -> bool {
        self.list_requests.requested.len() > 0 || self.list_requests.wanted.len() > 0
    }

    /// Whether lists are in flight.
    #[func]
    pub fn lists_in_flight(&self) -> bool {
        self.list_requests.requested.len() > 0
    }

    /// Whether patch `key`'s list arrived or was refused.
    #[func]
    pub fn list_known(&self, key: Vector2i) -> bool {
        let key = (key.x, key.y);
        self.state.lists.contains_key(&key) || self.state.lists_refused.contains_key(&key)
    }

    /// Patch `key`'s listed tiles, flat `(x, z)` (empty: none or not arrived).
    #[func]
    pub fn list_tiles(&self, key: Vector2i) -> PackedInt32Array {
        self.state
            .lists
            .get(&(key.x, key.y))
            .map(|(_, tiles)| tiles.iter().flat_map(|tile| [tile.x, tile.z]).collect())
            .unwrap_or_default()
    }

    /// Whether patch `key` must not draw its plane by T53 (a): its list has not arrived (nor been
    /// refused) or names a tile that can show (not refused, its pool not exhausted).
    #[func]
    pub fn list_drawable(&self, key: Vector2i) -> bool {
        let key = (key.x, key.y);
        match self.state.lists.get(&key) {
            None => !self.state.lists_refused.contains_key(&key),
            Some((_, tiles)) => tiles.iter().any(|&tile| !self.state.refused_at_level(tile)),
        }
    }

    /// Patch lists refused by contract.
    #[func]
    pub fn lists_refused_count(&self) -> i64 {
        self.state.lists_refused.len() as i64
    }

    /// This frame's lists taken and those equal to their patch's last (reset with the frame's
    /// counters).
    #[func]
    pub fn frame_list_counts(&self) -> Vector2i {
        Vector2i::new(
            self.list_requests.frame[0] as i32,
            self.list_requests.frame[1] as i32,
        )
    }

    /// Requests refused by contract (lists and tiles) since the last read.
    #[func]
    pub fn take_bake_failures(&mut self) -> i64 {
        std::mem::take(&mut self.list_requests.failures)
    }
}
