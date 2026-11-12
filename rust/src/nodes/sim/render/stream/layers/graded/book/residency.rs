// SPDX-License-Identifier: GPL-2.0-only

//! The resident patches handed to the book by the streaming core (`RENDER-32-K` part 6,
//! `docs/terrain.md` "Streaming core"): the terrain layer's residency, mesh steps and surface
//! generations ([`Follow`], `stream_node.rs`) set the book's resident entries directly, with each
//! patch's rectangle from the patch grid, instead of `terrain.gd` keeping a resident dictionary
//! and the renderer copying it in. The keys changed since the renderer's last frame are its
//! news (`take_patch_news`), which the frame follows as it followed the changed keys. The Godot
//! objects a patch draws with (its node, material and culling box) stay the executor's
//! (`terrain.gd` registers them with the renderer).
//!
//! A trace records each entry the core sets as a `patch_core` input (a replay sets it again
//! through [`GradedTileBook::core_patch`]). The renderer's `patch_mutated` (tests, replays of
//! earlier traces) sets entries without news, as before. Every call `O(1)`.

use super::super::state::PatchKey;
use super::*;
use crate::nodes::sim::render::stream::layers::terrain::Follow;

impl GradedTileBook {
    /// A terrain patch's change from the streaming core: `rect` its payload rectangle. A step
    /// change or a generation of a patch the book does not hold resident changes nothing.
    pub(crate) fn follow_terrain(&mut self, key: [i32; 2], follow: Follow, rect: [f32; 4]) {
        let key = (key[0], key[1]);
        match follow {
            Follow::Resident { step, generation } => {
                let patch = Patch {
                    rect,
                    step: i32::from(step),
                    generation,
                };
                self.follow_patch(key, Some(patch));
            }
            Follow::Step(step) => {
                if let Some(&patch) = self.state.resident.get(&key) {
                    self.follow_patch(
                        key,
                        Some(Patch {
                            step: i32::from(step),
                            ..patch
                        }),
                    );
                }
            }
            Follow::Left => {
                if self.state.resident.contains_key(&key) {
                    self.follow_patch(key, None);
                }
            }
        }
    }

    /// A terrain patch committed a payload of surface `generation` (`terrain.gd`'s commit,
    /// through the stream node; -1: its node was freed): a resident entry takes it. Any patch's
    /// commit is news, resident or not, as every commit was a changed key of the renderer's
    /// frame before the hand-off (a hidden patch's releases its blocks' parked tiles and evicts
    /// the commit-only tiles no resident patch meets).
    pub(crate) fn follow_generation(&mut self, key: [i32; 2], generation: i64) {
        let key = (key[0], key[1]);
        match self.state.resident.get(&key) {
            Some(&patch) => self.follow_patch(
                key,
                Some(Patch {
                    generation,
                    ..patch
                }),
            ),
            None => {
                self.patch_news.insert(key, ());
            }
        }
    }

    // Sets `key`'s resident entry (or erases it) as news for the renderer's next frame; a trace
    // records it.
    fn follow_patch(&mut self, key: PatchKey, patch: Option<Patch>) {
        self.state.set_patch(key, patch);
        self.patch_news.insert(key, ());
        let Some(trace) = self.trace.as_mut() else {
            return;
        };
        if trace.get("replaying").to::<bool>() {
            return;
        }
        let line = match patch {
            Some(patch) => {
                let rect: String = patch
                    .rect
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                format!(
                    "patch_core {} {} {rect} {} {}",
                    key.0, key.1, patch.step, patch.generation
                )
            }
            None => format!("patch_core_erase {} {}", key.0, key.1),
        };
        trace.call("entry", &[GString::from(line.as_str()).to_variant()]);
    }
}

#[godot_api(secondary)]
impl GradedTileBook {
    /// A recorded `patch_core` input replayed: `key` resident (its rectangle, step and
    /// generation) or erased, as the streaming core set it.
    #[func]
    pub fn core_patch(
        &mut self,
        key: Vector2i,
        resident: bool,
        rect: Rect2,
        step: i64,
        generation: i64,
    ) {
        let patch = resident.then_some(Patch {
            rect: [rect.position.x, rect.position.y, rect.size.x, rect.size.y],
            step: step as i32,
            generation,
        });
        self.follow_patch((key.x, key.y), patch);
    }

    /// Whether the core changed a resident entry since the last `take_patch_news`.
    #[func]
    pub fn has_patch_news(&self) -> bool {
        self.patch_news.len() > 0
    }

    /// The patches whose resident entry the core changed since the last call, flat `(x, z)` in
    /// the order first changed. O(news).
    #[func]
    pub fn take_patch_news(&mut self) -> PackedInt32Array {
        let out = self
            .patch_news
            .keys()
            .flat_map(|key| [key.0, key.1])
            .collect();
        self.patch_news.clear();
        out
    }

    /// Whether `key` is resident.
    #[func]
    pub fn is_resident(&self, key: Vector2i) -> bool {
        self.state.resident.contains_key(&(key.x, key.y))
    }

    /// Resident `key`'s surface generation (-1 when it is not resident).
    #[func]
    pub fn patch_generation(&self, key: Vector2i) -> i64 {
        self.state
            .resident
            .get(&(key.x, key.y))
            .map_or(-1, |patch| patch.generation)
    }

    /// Resident `key`'s rectangle (empty when it is not resident).
    #[func]
    pub fn patch_rect(&self, key: Vector2i) -> Rect2 {
        self.state
            .resident
            .get(&(key.x, key.y))
            .map_or(Rect2::default(), |patch| {
                Rect2::new(
                    Vector2::new(patch.rect[0], patch.rect[1]),
                    Vector2::new(patch.rect[2], patch.rect[3]),
                )
            })
    }
}
