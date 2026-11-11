// SPDX-License-Identifier: GPL-2.0-only

//! The graded pages' allocator (`RENDER-32-I`, `docs/terrain.md` "Pages and the row atlas"),
//! moved from `terrain_graded_pages.gd` with its behaviour: per level code the slot layer's side,
//! the most slots a page holds and the most atlas layers a tile's rows take; pages created within
//! the graded page budget, freed pages pooled by level and size for reuse; a page's free slots
//! and its free atlas runs (first fit, merged on free), a copy's placement, and a block move's
//! layout. Godot keeps the textures: every creation, reuse, release, drop and move is an event
//! the GDScript executor applies to the page's two `Texture2DArray`s, and every slot it writes is
//! the caller's (the copies).
//!
//! Bounds: a create, release or move `O(slots + layers)` of the pages involved (the free slot
//! list), a placement or vacate `O(the page's free runs)`, a pooled drop `O(size classes)`.
//! Nothing allocates once the pages exist but a page's own lists.
//!
//! `TERRAIN-09`: the pool kept up to [`FREE_PAGES_KEPT`] free pages of every size, each size a
//! block's page took, without a bound on their sum (11 pooled empty pages held 94.2 MiB at 100k,
//! T56). [`PoolPolicy::kept_bytes`] bounds the pooled pages' bytes ([`POOL_KEPT_BYTES`]): a
//! release past it frees the page instead of pooling it. What a page holds and draws does not
//! depend on whether it was pooled (a page's lists start fresh either way).

use super::ordered::OrderedMap;

/// Texels of a row atlas layer (`residency.rs` `ATLAS_WIDTH`).
pub(crate) const ATLAS_WIDTH: u64 = 256;
/// Bytes of one row atlas layer (RGBAF texels).
pub(crate) const ATLAS_LAYER_BYTES: u64 = ATLAS_WIDTH * 16;
/// The graded pages' memory budget, every page's texel and atlas layers (pooled free ones
/// included): a page the budget refuses leaves its tiles exhausted (counted, T53 (a)'s rule).
pub(crate) const PAGE_BUDGET_BYTES: u64 = 1024 * 1_048_576;
/// Free pages kept per level and size for reuse before a new one is created.
pub(crate) const FREE_PAGES_KEPT: usize = 4;
/// The pooled free pages' bytes together (`TERRAIN-09`). Block pages come in many sizes, so the
/// pool rarely serves one: on the recorded traces it held 101-119 MiB after the 30k and 100k
/// loads and the populated swing, and served 12 creations on the 30k load and 66 on Kuopio's
/// swing, every one of which fits under 16 MiB.
pub(crate) const POOL_KEPT_BYTES: u64 = 32 * 1_048_576;

/// A copy's place in a page: its slot layer, and its row atlas run (first layer, layers).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct Copy {
    pub slot: u32,
    pub first: u32,
    pub layers: u32,
}

/// A level's page shape.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LevelShape {
    // Texels per side of a slot layer (side x (side + 1), the last line the header).
    pub side: u64,
}

/// How many freed pages the pool keeps.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PoolPolicy {
    // Per level and size.
    pub kept_per_size: usize,
    // Their bytes together (`TERRAIN-09`); `None`: no bound but the page budget.
    pub kept_bytes: Option<u64>,
}

impl Default for PoolPolicy {
    fn default() -> Self {
        Self {
            kept_per_size: FREE_PAGES_KEPT,
            kept_bytes: Some(POOL_KEPT_BYTES),
        }
    }
}

impl PoolPolicy {
    /// The pool before `TERRAIN-09`: four a size, no bound on their bytes (the GDScript
    /// allocator's, which the recorded traces replay).
    #[cfg(test)]
    pub(crate) fn unbounded() -> Self {
        Self {
            kept_per_size: FREE_PAGES_KEPT,
            kept_bytes: None,
        }
    }
}

/// One page: `slots` slot layers and `layers` atlas layers of level `code`, for `block`.
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub code: u8,
    pub block: (i32, i32),
    pub slots: u32,
    pub layers: u32,
    pub bytes: u64,
    // Free slots: the next taken is the last.
    free: Vec<u32>,
    // Free atlas runs (first, count), sorted by first, merged.
    runs: Vec<(u32, u32)>,
}

/// A page's free slots and runs, as a reservation keeps them to put back.
#[derive(Clone, Debug)]
pub(crate) struct PageLists {
    free: Vec<u32>,
    runs: Vec<(u32, u32)>,
}

impl Page {
    pub(crate) fn lists(&self) -> PageLists {
        PageLists {
            free: self.free.clone(),
            runs: self.runs.clone(),
        }
    }

    pub(crate) fn set_lists(&mut self, lists: PageLists) {
        self.free = lists.free;
        self.runs = lists.runs;
    }

    pub(crate) fn free_slots(&self) -> usize {
        self.free.len()
    }

    pub(crate) fn free_layers(&self) -> u32 {
        self.runs.iter().map(|run| run.1).sum()
    }

    // The first free run at least `count` layers long. O(runs).
    fn find_run(&self, count: u32) -> Option<usize> {
        self.runs.iter().position(|run| run.1 >= count)
    }

    /// Whether a free run of `count` layers exists. O(runs).
    pub(crate) fn has_run(&self, count: u32) -> bool {
        self.find_run(count).is_some()
    }

    // Takes `count` layers from the first free run long enough. O(runs).
    fn take_run(&mut self, count: u32) -> Option<u32> {
        let index = self.find_run(count)?;
        let (first, length) = self.runs[index];
        if length == count {
            self.runs.remove(index);
        } else {
            self.runs[index] = (first + count, length - count);
        }
        Some(first)
    }

    // Returns layers [first, first + count) to the free runs, merged with its neighbours.
    // O(runs).
    fn free_run(&mut self, first: u32, count: u32) {
        let index = self.runs.partition_point(|run| run.0 < first);
        self.runs.insert(index, (first, count));
        if index + 1 < self.runs.len() && first + count == self.runs[index + 1].0 {
            self.runs[index].1 += self.runs[index + 1].1;
            self.runs.remove(index + 1);
        }
        if index > 0 && self.runs[index - 1].0 + self.runs[index - 1].1 == first {
            self.runs[index - 1].1 += self.runs[index].1;
            self.runs.remove(index);
        }
    }

    /// Whether the page can take a copy of `layers` atlas layers, `held` its copy here: a free
    /// slot for a new copy, and a run unless the held one is long enough.
    pub(crate) fn can_place(&self, held: Option<Copy>, layers: u32) -> bool {
        if held.is_none() && self.free.is_empty() {
            return false;
        }
        let new_run = layers > 0 && held.is_none_or(|copy| layers > copy.layers);
        !(new_run && !self.has_run(layers))
    }

    /// Places a copy (`can_place` holds): the held slot kept, else a free one, and the held run
    /// kept while long enough, else a new one (the held run freed after).
    pub(crate) fn place(&mut self, held: Option<Copy>, layers: u32) -> Copy {
        let mut copy = held.unwrap_or(Copy {
            slot: u32::MAX,
            first: 0,
            layers: 0,
        });
        if layers > 0 && held.is_none_or(|held| layers > held.layers) {
            let first = self
                .take_run(layers)
                .expect("can_place found a run long enough");
            if let Some(held) = held.filter(|held| held.layers > 0) {
                self.free_run(held.first, held.layers);
            }
            copy.first = first;
            copy.layers = layers;
        }
        if held.is_none() {
            copy.slot = self.free.pop().expect("can_place found a free slot");
        }
        copy
    }

    /// Returns a copy's slot and run.
    pub(crate) fn vacate(&mut self, copy: Copy) {
        self.free.push(copy.slot);
        if copy.layers > 0 {
            self.free_run(copy.first, copy.layers);
        }
    }

    // A fresh page's lists: every slot free (slot 0 taken first), one run of every layer.
    fn reset_lists(&mut self) {
        self.free.clear();
        self.free.extend((0..self.slots).rev());
        self.runs.clear();
        self.runs.push((0, self.layers));
    }
}

/// What the textures must follow, in order (the GDScript executor applies them).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageEvent {
    /// A page was created: its two arrays are made, zeroed.
    New(u32),
    /// A pooled page was taken for a new block (its arrays kept).
    Reuse(u32),
    /// A page with no copy left was released: pooled when `kept`, else freed.
    Release { id: u32, kept: bool },
    /// A pooled page was freed to make room in the budget.
    Drop(u32),
    /// A block moved: `from`'s layers are copied into `to`.
    Copy { from: u32, to: u32 },
    /// The budget refused a page.
    Refused,
}

// A pool size class: level code, slots, layers.
type SizeKey = (u8, u32, u32);

/// The pages of the graded levels.
#[derive(Debug, Default)]
pub(crate) struct PageStore {
    levels: Vec<LevelShape>,
    pages: OrderedMap<u32, Page>,
    // Every page's bytes, pooled free ones included.
    bytes: u64,
    // The pooled ones'.
    pooled_bytes: u64,
    next_id: u32,
    // Freed pages per size class, the classes in the order they were first released into.
    free: OrderedMap<SizeKey, Vec<u32>>,
    policy: PoolPolicy,
    events: Vec<PageEvent>,
}

impl PageStore {
    /// Adds the next level's shape.
    pub(crate) fn add_level(&mut self, side: u64) {
        self.levels.push(LevelShape { side });
    }

    pub(crate) fn set_policy(&mut self, policy: PoolPolicy) {
        self.policy = policy;
    }

    pub(crate) fn level(&self, code: u8) -> &LevelShape {
        &self.levels[usize::from(code)]
    }

    #[cfg(test)]
    pub(crate) fn levels(&self) -> usize {
        self.levels.len()
    }

    pub(crate) fn page(&self, id: u32) -> Option<&Page> {
        self.pages.get(&id)
    }

    pub(crate) fn page_mut(&mut self, id: u32) -> Option<&mut Page> {
        self.pages.get_mut(&id)
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(crate) fn pooled_bytes(&self) -> u64 {
        self.pooled_bytes
    }

    /// The events since the last take, in order.
    pub(crate) fn take_events(&mut self, out: &mut Vec<PageEvent>) {
        out.append(&mut self.events);
    }

    /// Drops every page, the pooled ones included (a renderer reset; no event: the executor
    /// drops its arrays with it), and without `keep_levels` the levels too.
    pub(crate) fn reset(&mut self, keep_levels: bool) {
        if !keep_levels {
            self.levels.clear();
        }
        self.pages.clear();
        self.free.clear();
        self.bytes = 0;
        self.pooled_bytes = 0;
        self.events.clear();
    }

    /// A page's bytes: its slot layers and atlas layers.
    pub(crate) fn page_bytes(&self, code: u8, slots: u32, layers: u32) -> u64 {
        let side = self.level(code).side;
        u64::from(slots) * side * (side + 1) * 16 + u64::from(layers) * ATLAS_LAYER_BYTES
    }

    /// A page of `slots` slot layers and `layers` atlas layers for `block` at level `code`: a
    /// pooled free one of that size, else created within the budget (pooled pages dropped
    /// first as it needs room). `None`: the budget refuses it. O(slots + layers).
    pub(crate) fn create(&mut self, code: u8, block: (i32, i32), slots: u32, layers: u32) -> Option<u32> {
        let size = (code, slots, layers);
        let pooled = self.free.get_mut(&size).and_then(Vec::pop);
        let id = match pooled {
            Some(id) => {
                let page_bytes = self.pages.get(&id).expect("pooled pages exist").bytes;
                self.pooled_bytes -= page_bytes;
                self.events.push(PageEvent::Reuse(id));
                id
            }
            None => {
                let page_bytes = self.page_bytes(code, slots, layers);
                while self.bytes + page_bytes > PAGE_BUDGET_BYTES && self.drop_free_page() {}
                if self.bytes + page_bytes > PAGE_BUDGET_BYTES {
                    self.events.push(PageEvent::Refused);
                    return None;
                }
                let id = self.next_id;
                self.next_id += 1;
                self.pages.insert(
                    id,
                    Page {
                        code,
                        block,
                        slots,
                        layers,
                        bytes: page_bytes,
                        free: Vec::with_capacity(slots as usize),
                        runs: Vec::new(),
                    },
                );
                self.bytes += page_bytes;
                self.events.push(PageEvent::New(id));
                id
            }
        };
        let page = self.pages.get_mut(&id).expect("just made or pooled");
        page.block = block;
        page.reset_lists();
        Some(id)
    }

    // Frees one pooled page, from the first size class (in the order the classes were first
    // released into) holding one. Returns whether one was.
    fn drop_free_page(&mut self) -> bool {
        let found = self
            .free
            .iter()
            .find_map(|(size, pooled)| pooled.last().map(|&id| (*size, id)));
        let Some((size, id)) = found else {
            return false;
        };
        self.free.get_mut(&size).expect("found it").pop();
        let page = self.pages.remove(&id).expect("pooled pages exist");
        self.bytes -= page.bytes;
        self.pooled_bytes -= page.bytes;
        self.events.push(PageEvent::Drop(id));
        true
    }

    /// Returns a page holding no copy to the pool, or frees it past the pool's policy.
    pub(crate) fn release(&mut self, id: u32) {
        let page = self.pages.get(&id).expect("released pages exist");
        let (size, page_bytes) = ((page.code, page.slots, page.layers), page.bytes);
        if !self.free.contains_key(&size) {
            self.free.insert(size, Vec::new());
        }
        let pooled = self.free.get_mut(&size).expect("just ensured");
        let kept = pooled.len() < self.policy.kept_per_size
            && self
                .policy
                .kept_bytes
                .is_none_or(|limit| self.pooled_bytes + page_bytes <= limit);
        self.events.push(PageEvent::Release { id, kept });
        if kept {
            pooled.push(id);
            self.pooled_bytes += page_bytes;
            return;
        }
        self.pages.remove(&id);
        self.bytes -= page_bytes;
    }

    /// Lays out `target` (a page just made or pooled, holding no copy) as a move of `source`'s
    /// copies: the same slots and runs taken, its extra slots and layers free, the source's bytes
    /// copied by the executor. O(slots + runs).
    pub(crate) fn copy_layout(&mut self, source: u32, target: u32) {
        self.events.push(PageEvent::Copy {
            from: source,
            to: target,
        });
        let from = self.pages.get(&source).expect("moved pages exist");
        let (slots, layers, free, runs) = (from.slots, from.layers, from.free.clone(), from.runs.clone());
        let to = self.pages.get_mut(&target).expect("moved pages exist");
        to.free.clear();
        to.free.extend((slots..to.slots).rev());
        to.free.extend_from_slice(&free);
        to.runs = runs;
        if to.layers > layers {
            to.free_run(layers, to.layers - layers);
        }
    }
}

#[cfg(test)]
mod tests;
