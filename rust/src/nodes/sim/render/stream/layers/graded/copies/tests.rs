// SPDX-License-Identifier: GPL-2.0-only

//! The copies' rules on a small world: a block's first page sized for its lists, placement and
//! its effects, a full page's growth (a move keeping every copy) and an emptied page's release.

use super::super::pages::{PageEvent, PoolPolicy};
use super::super::state::{Effect, Entry, Graded, LevelCaps, PageRef, Shape};
use super::*;

// A 2 x 2-patch world of 510 m patches from (-510, -510), blocks of 2 patches, the GDScript
// renderer's GL0-GL3 shapes with GL2's cap small.
fn world() -> Graded {
    let mut graded = Graded {
        shape: Shape {
            world_min: [-510.0, -510.0],
            patch_span: 510.0,
            patch_grid: [2, 2],
            block_patches: 2,
            layer_limit: 2048,
        },
        ..Graded::default()
    };
    graded.pages.set_policy(PoolPolicy::unbounded());
    for (side, cap, max_layers) in [(131, 64, 24), (67, 192, 18), (35, 96, 9), (35, 256, 9)] {
        graded.pages.add_level(side);
        graded.caps.push(LevelCaps { cap, max_layers });
    }
    graded.resident.insert((0, 0), super::super::cells::Patch::default());
    graded
}

fn tile(x: i32, z: i32) -> Tile {
    Tile { x, z }
}

// Installs `key` at GL2 with its placed copies, as `_upload` records them.
fn install(graded: &mut Graded, key: Tile, layers: u32, scratch: &mut Vec<(PageRef, Block)>) -> Result<(), PlaceFailure> {
    let held = graded.installed.get(&key).map(|entry| entry.copies.clone()).unwrap_or_default();
    let copies = graded.place_copies(key, 2, layers, &held, false, false, scratch)?;
    graded.installed.insert(
        key,
        Entry {
            cell_code: 2,
            level: 2,
            step: 0,
            token: 0,
            behind: false,
            copies,
            adoptable: true,
            walls: Vec::new(),
        },
    );
    Ok(())
}

#[test]
fn a_block_page_waits_for_lists_then_fits_them() {
    let mut graded = world();
    let mut scratch = Vec::new();
    // Patch (0, 0)'s list has not arrived: its block's page waits.
    graded.copy_pages(tile(-8, -8), 2, &mut scratch);
    assert_eq!(scratch, vec![(PageRef::Pending, (0, 0))]);
    assert_eq!(install(&mut graded, tile(-8, -8), 2, &mut scratch), Err(PlaceFailure::Pending));
    assert!(graded.wanted.contains_key(&tile(-8, -8)));
    // Its list in: a page of its tiles plus an eighth and 8, rounded up to 32 slots, three
    // layers a slot; bound to the block's sets, then written.
    graded.lists.insert((0, 0), (0, (0..40).map(|at| tile(-8 + at % 8, -8 + at / 8)).collect()));
    graded.effects.clear();
    install(&mut graded, tile(-8, -8), 2, &mut scratch).unwrap();
    let page = *graded.block_page[2].get(&(0, 0)).unwrap();
    let live = graded.pages.page(page).unwrap();
    assert_eq!((live.slots, live.layers), (64, 192));
    assert_eq!(
        graded.effects,
        vec![
            Effect::PageMade { id: page, reused: false, code: 2, block: (0, 0), slots: 64, layers: 192 },
            Effect::Bind { code: 2, block: (0, 0), rewrite: false, page: Some(page) },
            Effect::Write { page, copy: Copy { slot: 0, first: 0, layers: 2 } },
        ]
    );
}

#[test]
fn a_full_page_grows_and_keeps_every_copy() {
    let mut graded = world();
    let mut scratch = Vec::new();
    graded.lists.insert((0, 0), (0, vec![tile(-8, -8)]));
    // 1 listed tile: 32 slots. Fill them.
    for at in 0..32 {
        install(&mut graded, tile(-8 + at % 8, -8 + at / 8), 1, &mut scratch).unwrap();
    }
    let bound = *graded.block_page[2].get(&(0, 0)).unwrap();
    graded.effects.clear();
    // The 33rd waits for the move the full page starts: twice the slots, the layout copied.
    let waiting = tile(-8, -4);
    assert_eq!(install(&mut graded, waiting, 1, &mut scratch), Err(PlaceFailure::Waits));
    let target = *graded.block_target[2].get(&(0, 0)).unwrap();
    assert_eq!(graded.effects[0], Effect::PageMade { id: target, reused: false, code: 2, block: (0, 0), slots: 64, layers: 192 });
    assert_eq!(graded.effects[1], Effect::Grew { code: 2, block: (0, 0), slots: 64, layers: 192 });
    assert_eq!(graded.effects[2], Effect::Page(PageEvent::Copy { from: bound, to: target }));
    assert_eq!(graded.effects.len(), 3 + 32, "each moved tile's entry is reported");
    assert_eq!(graded.pages.page(target).unwrap().slots, 64);
    assert_eq!(graded.growth_waits.get(&waiting), Some(&(2, (0, 0))));
    for (_, entry) in graded.installed.iter() {
        assert_eq!(entry.copy_in(bound), entry.copy_in(target), "a move keeps each copy's place");
    }
    // The switch binds the target, releases the old page and asks the waiting tile again.
    graded.effects.clear();
    graded.wanted.clear();
    graded.switch_block(2, (0, 0));
    assert_eq!(graded.block_page[2].get(&(0, 0)), Some(&target));
    assert!(graded.installed.iter().all(|(_, entry)| entry.copies.len() == 1));
    assert_eq!(graded.effects[32], Effect::Page(PageEvent::Release { id: bound, kept: true }));
    assert_eq!(graded.effects[33], Effect::Bind { code: 2, block: (0, 0), rewrite: true, page: Some(target) });
    assert_eq!(graded.effects[34], Effect::Moved);
    assert!(graded.wanted.contains_key(&waiting));
    install(&mut graded, waiting, 1, &mut scratch).unwrap();
}

#[test]
fn the_last_copy_out_releases_a_block_page() {
    let mut graded = world();
    let mut scratch = Vec::new();
    graded.lists.insert((0, 0), (0, vec![tile(-8, -8)]));
    install(&mut graded, tile(-8, -8), 1, &mut scratch).unwrap();
    let page = *graded.block_page[2].get(&(0, 0)).unwrap();
    graded.effects.clear();
    graded.free_copies(tile(-8, -8), false);
    assert_eq!(graded.block_page[2].get(&(0, 0)), None);
    assert_eq!(
        graded.effects,
        vec![
            Effect::Page(PageEvent::Release { id: page, kept: true }),
            Effect::Bind { code: 2, block: (0, 0), rewrite: false, page: None },
        ]
    );
}
