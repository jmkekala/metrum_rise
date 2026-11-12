// SPDX-License-Identifier: GPL-2.0-only

//! The graded renderer's state as `terrain_graded_tiles.gd` keeps it (`RENDER-32-I`, WIP: not
//! wired yet). One struct, its rules split by domain: [`super::copies`] (a block's pages, a tile's
//! copies, growth), the tiles' requests and the hand-over (next). Every collection the GDScript
//! iterated where the order reached an output is an [`OrderedMap`] (a Godot `Dictionary`'s
//! order); the rest are hash maps.
//!
//! The GDScript renderer keeps the cells, the edit batch's staged results and the preview's
//! waits; what the state decides for them comes back as [`Effect`]s in the order the GDScript
//! made the same calls, which it applies after each call.

use super::ordered::OrderedMap;
use super::pages::{Copy, PageEvent, PageStore};
use crate::simulation::terrain::graded::TileKey;

/// A render patch's key (column, row of the patch grid).
pub(crate) type PatchKey = (i32, i32);
/// A block of patches at a level (the world's `(0, 0)` at `GL0` and `GL1`).
pub(crate) type Block = (i32, i32);
/// A tile's key.
pub(crate) type Tile = TileKey;

/// The tile side in metres, as the GDScript multiplies it (`real_t`).
pub(crate) const TILE_M: f32 = 64.0;
/// Atlas layers a block page holds per slot.
pub(crate) const PAGE_LAYERS_PER_SLOT: u32 = 3;

/// What a tile draws at a level: installed or a preview's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    // The cell mesh's lattice code (the level below `GL3`, else 3 + log2(step)).
    pub cell_code: u8,
    pub level: u8,
    pub step: u8,
    pub token: i64,
    pub behind: bool,
    // Its copy in each page, in the order they were placed (a free walks them in this order).
    pub copies: Vec<(u32, Copy)>,
    // A preview entry the commit may adopt (false once a later pose of it failed to place).
    pub adoptable: bool,
}

impl Entry {
    pub(crate) fn copy_in(&self, page: u32) -> Option<Copy> {
        self.copies
            .iter()
            .find(|(held, _)| *held == page)
            .map(|(_, copy)| *copy)
    }
}

/// Where one of a tile's copies goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageRef {
    Page(u32),
    // The block's page waits for its patches' lists (GDScript `PAGE_PENDING`).
    Pending,
    // The budget refused the block's first page.
    Refused,
}

/// What the GDScript renderer does after a call, in order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    /// A page event (the page store's arrays): releases, drops, copies, refusals.
    Page(PageEvent),
    /// A page was made (`reused`: taken from the pool) for `block` at level `code`.
    PageMade { id: u32, reused: bool, code: u8, block: Block, slots: u32, layers: u32 },
    /// A block started a move to a larger page (counted, logged).
    Grew { code: u8, block: Block, slots: u32, layers: u32 },
    /// A block switched to its larger page (counted).
    Moved,
    /// Upload the tile being placed into `page` at `copy`.
    Write { page: u32, copy: Copy },
    /// Block `block`'s sets bind its page at level `code` (`_bind_block`), `page` then.
    Bind { code: u8, block: Block, rewrite: bool, page: Option<u32> },
    /// Cancel `tile`'s pending bake; a `stale` cancel's answer goes back (`cancel_results`).
    Cancel { tile: Tile, stale: bool },
    /// `tile`'s level's pool is exhausted (counted, logged; its patches' sets redraw).
    Exhausted { tile: Tile, code: u8 },
    /// `tile` was evicted (counted; its cell and walls redraw; the mirror drops it).
    Evicted { tile: Tile },
    /// `tile`'s staged edit result is dropped.
    EditDropped { tile: Tile },
    /// An upload rewrote a tile at its installed level (counted).
    SameLevel,
    /// `tile`'s entry changed (the mirror takes it from the book).
    Installed { tile: Tile, preview: bool },
    /// `tile`'s grass mask must render again.
    MaskChanged { tile: Tile },
    /// `tile`'s cell instance changed, with `sides` its neighbours' sides.
    CellChanged { tile: Tile, sides: bool },
    /// The sets of the patches meeting `tile` start or stop drawing, or build first.
    AwaitCells { tile: Tile },
}

/// The patch grid and the pages' shapes (setup).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Shape {
    // World XZ of the patch grid's lower corner (`world_min`, real_t).
    pub world_min: [f32; 2],
    // A patch's side in metres; GDScript divides `real_t` vectors by it, as `real_t`.
    pub patch_span: f32,
    // Patches per axis.
    pub patch_grid: [i32; 2],
    // A block's side in patches at `GL2` and `GL3`.
    pub block_patches: i32,
    // The device's array layer limit.
    pub layer_limit: u32,
}

/// Per level code: the most slots a page holds and the most atlas layers a tile's rows take.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LevelCaps {
    pub cap: u32,
    pub max_layers: u32,
}

/// A page's holders: the installed tiles and the preview tiles with a copy there.
pub(crate) type Holders = (OrderedMap<Tile, ()>, OrderedMap<Tile, ()>);

/// The state.
#[derive(Default)]
pub(crate) struct Graded {
    pub(crate) shape: Shape,
    pub(crate) caps: Vec<LevelCaps>,
    pub(crate) pages: PageStore,
    // Per level code: block -> the page its patches' sets bind; block -> the larger page a growing
    // block moves to; block -> tiles waiting for the move.
    pub(crate) block_page: [OrderedMap<Block, u32>; 4],
    pub(crate) block_target: [OrderedMap<Block, u32>; 4],
    pub(crate) block_waiting: [OrderedMap<Block, OrderedMap<Tile, ()>>; 4],
    // Growing blocks to check, and the tiles waiting for a move -> its (code, block).
    pub(crate) growth_dirty: OrderedMap<(u8, Block), ()>,
    pub(crate) growth_waits: OrderedMap<Tile, (u8, Block)>,
    // Per page: the installed and preview tiles holding a copy there.
    pub(crate) holders: std::collections::HashMap<u32, Holders>,
    pub(crate) installed: OrderedMap<Tile, Entry>,
    pub(crate) preview: OrderedMap<Tile, Entry>,
    // The resident patches, and per patch its list (token, tiles) or its refused list's token.
    pub(crate) resident: OrderedMap<PatchKey, ()>,
    pub(crate) lists: OrderedMap<PatchKey, (i64, Vec<Tile>)>,
    pub(crate) lists_refused: OrderedMap<PatchKey, i64>,
    // The load's settle flags (`unbudgeted`, `residency_settling`).
    pub(crate) unbudgeted: bool,
    pub(crate) residency_settling: bool,
    // The tiles that want a request (GDScript `_wanted`).
    pub(crate) wanted: OrderedMap<Tile, ()>,
    pub(crate) effects: Vec<Effect>,
    // The desired levels and their travel buckets, and the requests' half.
    pub(crate) levels: crate::nodes::sim::render::stream::levels::LevelBook,
    pub(crate) requests: super::tiles::Requests,
    // Scratch for the wanted tiles a hand-over takes.
    pub(crate) wanted_scratch: Vec<Tile>,
    // A trace records the hand-over's stale cancels in their place among the effects.
    pub(crate) markers: bool,
    // Scratch for a block's listed tiles.
    pub(crate) listed_scratch: std::collections::HashSet<Tile>,
}

impl Graded {
    // Moves the page store's events into the effects, in order.
    pub(crate) fn page_events(&mut self) {
        let mut events = Vec::new();
        self.pages.take_events(&mut events);
        for event in events {
            let made = match event {
                PageEvent::New(id) => Some((id, false)),
                PageEvent::Reuse(id) => Some((id, true)),
                _ => None,
            };
            let effect = match made.and_then(|(id, reused)| self.pages.page(id).map(|page| (id, reused, page))) {
                Some((id, reused, page)) => Effect::PageMade {
                    id,
                    reused,
                    code: page.code,
                    block: page.block,
                    slots: page.slots,
                    layers: page.layers,
                },
                None => Effect::Page(event),
            };
            self.effects.push(effect);
        }
    }

    pub(crate) fn holders_mut(&mut self, page: u32) -> &mut Holders {
        self.holders.entry(page).or_default()
    }

    pub(crate) fn holders_empty(&self, page: u32) -> bool {
        self.holders
            .get(&page)
            .is_none_or(|(tiles, previews)| tiles.len() == 0 && previews.len() == 0)
    }

    pub(crate) fn entries_mut(&mut self, is_preview: bool) -> &mut OrderedMap<Tile, Entry> {
        if is_preview { &mut self.preview } else { &mut self.installed }
    }
}
