// SPDX-License-Identifier: GPL-2.0-only

//! `GradedTileBook`, the graded tile renderer's state in Rust as `terrain_graded_tiles.gd` owns
//! it (`RENDER-32-I`). So far the page allocator ([`super::pages`]): the GDScript renderer keeps
//! each page's textures and holders and applies the page events after each call, and asks the
//! book where a copy goes. Pages are ids; a copy is `Vector3i(slot, first atlas layer, layers)`,
//! `(-1, 0, 0)` for none.

use super::pages::{Copy, PageEvent, PageStore, PoolPolicy};
use godot::prelude::*;

// Event kinds of `take_page_events`, four integers each: kind, id, then per kind.
const EVENT_NEW: i64 = 0;
const EVENT_REUSE: i64 = 1;
const EVENT_RELEASE: i64 = 2;
const EVENT_DROP: i64 = 3;
const EVENT_COPY: i64 = 4;
const EVENT_REFUSED: i64 = 5;

fn copy_of(copy: Vector3i) -> Option<Copy> {
    (copy.x >= 0).then_some(Copy {
        slot: copy.x as u32,
        first: copy.y as u32,
        layers: copy.z as u32,
    })
}

fn copy_vector(copy: Copy) -> Vector3i {
    Vector3i::new(copy.slot as i32, copy.first as i32, copy.layers as i32)
}

/// The graded tile renderer's book (`terrain_graded_tiles.gd`, `TERRAIN-13` part 2): the page
/// allocator of its levels. Every page id the renderer passes must be live.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct GradedTileBook {
    pages: PageStore,
    // Pages a reservation changed, with their lists before it (plan_keep).
    plan: Vec<(u32, super::pages::PageLists)>,
    events: Vec<PageEvent>,
}

#[godot_api]
impl GradedTileBook {
    /// Adds the next level's page shape: its slot layer's `side` (a slot layer is `side` by
    /// `side + 1` RGBAF texels).
    #[func]
    pub fn add_level(&mut self, side: i64) {
        self.pages.add_level(side as u64);
    }

    /// Drops every page and level, as the renderer sets up.
    #[func]
    pub fn reset_pages(&mut self, keep_levels: bool) {
        self.pages.reset(keep_levels);
        self.plan.clear();
    }

    /// Bounds the pooled free pages' bytes (`TERRAIN-09`); negative: no bound but four a size.
    #[func]
    pub fn set_pool_kept_bytes(&mut self, bytes: i64) {
        self.pages.set_policy(PoolPolicy {
            kept_bytes: u64::try_from(bytes).ok(),
            ..PoolPolicy::default()
        });
    }

    /// A page of `slots` slot layers and `layers` atlas layers for `block` at level `code`: a
    /// pooled one of that size, else a new one within the budget. -1: the budget refuses it.
    /// O(slots + layers).
    #[func]
    pub fn create_page(&mut self, code: i64, block: Vector2i, slots: i64, layers: i64) -> i64 {
        self.pages
            .create(code as u8, (block.x, block.y), slots as u32, layers as u32)
            .map_or(-1, i64::from)
    }

    /// Returns a page holding no copy to the pool (or frees it past the pool's bound).
    #[func]
    pub fn release_page(&mut self, page: i64) {
        self.pages.release(page as u32);
    }

    /// Lays `target` (just made or pooled) out as the move of `source`'s copies.
    #[func]
    pub fn copy_layout(&mut self, source: i64, target: i64) {
        self.pages.copy_layout(source as u32, target as u32);
    }

    /// Whether `page` can take a copy of `layers` atlas layers, `held` its copy there.
    #[func]
    pub fn can_place(&self, page: i64, held: Vector3i, layers: i64) -> bool {
        self.page(page).can_place(copy_of(held), layers as u32)
    }

    /// Places a copy in `page` (`can_place` holds) and returns it. O(the page's free runs).
    #[func]
    pub fn place(&mut self, page: i64, held: Vector3i, layers: i64) -> Vector3i {
        let page = self.pages.page_mut(page as u32).expect("live page");
        copy_vector(page.place(copy_of(held), layers as u32))
    }

    /// Returns a copy's slot and run to `page`.
    #[func]
    pub fn vacate(&mut self, page: i64, copy: Vector3i) {
        let copy = copy_of(copy).expect("a copy");
        self.pages
            .page_mut(page as u32)
            .expect("live page")
            .vacate(copy);
    }

    /// `page`'s free slots.
    #[func]
    pub fn free_slots(&self, page: i64) -> i64 {
        self.page(page).free_slots() as i64
    }

    /// `page`'s free atlas layers.
    #[func]
    pub fn free_layers(&self, page: i64) -> i64 {
        i64::from(self.page(page).free_layers())
    }

    /// Whether `page` has a free atlas run of `layers` layers.
    #[func]
    pub fn has_run(&self, page: i64, layers: i64) -> bool {
        self.page(page).has_run(layers as u32)
    }

    /// Every page's bytes, the pooled ones included.
    #[func]
    pub fn page_bytes(&self) -> i64 {
        self.pages.bytes() as i64
    }

    /// The pooled free pages' bytes.
    #[func]
    pub fn pooled_bytes(&self) -> i64 {
        self.pages.pooled_bytes() as i64
    }

    /// A reservation (an edit batch's copies, all or none) starts: pages it places into are
    /// kept as they were first (plan_keep), to be put back by plan_abort.
    #[func]
    pub fn plan_begin(&mut self) {
        self.plan.clear();
    }

    /// Keeps `page`'s lists before the reservation's first placement in it.
    #[func]
    pub fn plan_keep(&mut self, page: i64) {
        let id = page as u32;
        if self.plan.iter().any(|(kept, _)| *kept == id) {
            return;
        }
        let lists = self.pages.page(id).expect("live page").lists();
        self.plan.push((id, lists));
    }

    /// The reservation failed: its pages get their lists back.
    #[func]
    pub fn plan_abort(&mut self) {
        for (id, lists) in self.plan.drain(..) {
            self.pages
                .page_mut(id)
                .expect("live page")
                .set_lists(lists);
        }
    }

    /// The page events since the last take, four integers each: kind (0 new, 1 reused, 2
    /// released, 3 dropped, 4 copied, 5 refused), page id, then kept (released) or the target
    /// (copied), and 0.
    #[func]
    pub fn take_page_events(&mut self) -> PackedInt64Array {
        self.pages.take_events(&mut self.events);
        let mut flat = Vec::with_capacity(self.events.len() * 4);
        for event in self.events.drain(..) {
            let entry = match event {
                PageEvent::New(id) => [EVENT_NEW, i64::from(id), 0, 0],
                PageEvent::Reuse(id) => [EVENT_REUSE, i64::from(id), 0, 0],
                PageEvent::Release { id, kept } => [EVENT_RELEASE, i64::from(id), i64::from(kept), 0],
                PageEvent::Drop(id) => [EVENT_DROP, i64::from(id), 0, 0],
                PageEvent::Copy { from, to } => [EVENT_COPY, i64::from(from), i64::from(to), 0],
                PageEvent::Refused => [EVENT_REFUSED, -1, 0, 0],
            };
            flat.extend_from_slice(&entry);
        }
        PackedInt64Array::from(flat.as_slice())
    }
}

impl GradedTileBook {
    fn page(&self, page: i64) -> &super::pages::Page {
        self.pages.page(page as u32).expect("live page")
    }
}
