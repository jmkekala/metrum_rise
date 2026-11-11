// SPDX-License-Identifier: GPL-2.0-only

//! The page allocator against the GDScript allocator's rules (`terrain_graded_pages.gd` before
//! `RENDER-32-I`): slot order, first-fit runs merged on free, the pool's reuse and drop order, the
//! budget, a move's layout and `TERRAIN-09`'s bound on the pooled pages.

use super::*;

// GL2-like shapes: a 17-texel side (17 x 18 x 16 bytes a slot) and three layers a tile.
fn store() -> PageStore {
    let mut store = PageStore::default();
    store.add_level(9);
    store.add_level(9);
    store.add_level(17);
    store
}

#[test]
fn slots_and_runs_follow_the_gdscript_order() {
    let mut store = store();
    let id = store.create(2, (0, 0), 4, 12).unwrap();
    let page = store.page_mut(id).unwrap();
    // Slot 0 first, runs first fit.
    let a = page.place(None, 3);
    let b = page.place(None, 2);
    let c = page.place(None, 0);
    assert_eq!((a.slot, a.first, a.layers), (0, 0, 3));
    assert_eq!((b.slot, b.first, b.layers), (1, 3, 2));
    assert_eq!((c.slot, c.first, c.layers), (2, 0, 0));
    // A held copy keeps its slot and run while long enough; a longer run is taken first, the
    // held one freed after.
    assert_eq!(page.place(Some(a), 2), a);
    let a3 = page.place(Some(a), 4);
    assert_eq!((a3.slot, a3.first, a3.layers), (0, 5, 4));
    assert_eq!(page.runs, vec![(0, 3), (9, 3)]);
    assert!(!page.can_place(None, 4));
    assert!(page.can_place(Some(a3), 4));
    // Vacating merges with both neighbours and pushes the slot last (taken next).
    page.vacate(b);
    assert_eq!(page.runs, vec![(0, 5), (9, 3)]);
    page.vacate(a3);
    assert_eq!(page.runs, vec![(0, 12)]);
    assert_eq!(page.free, vec![3, 1, 0]);
    assert_eq!(page.place(None, 1).slot, 0);
}

#[test]
fn the_pool_reuses_by_size_and_drops_in_class_order() {
    let mut store = store();
    let a = store.create(2, (0, 0), 32, 96).unwrap();
    let b = store.create(2, (1, 0), 64, 192).unwrap();
    let c = store.create(2, (2, 0), 32, 96).unwrap();
    let total = store.bytes();
    store.release(b);
    store.release(a);
    store.release(c);
    assert_eq!(store.bytes(), total, "pooled pages keep their bytes");
    // The last pooled of a size is taken first, with fresh lists and its new block.
    let d = store.create(2, (5, 5), 32, 96).unwrap();
    assert_eq!(d, c);
    let page = store.page(d).unwrap();
    assert_eq!((page.block, page.free_slots(), page.free_layers()), ((5, 5), 32, 96));
    // Past the budget, pooled pages drop from the first size class released into (b's).
    let mut events = Vec::new();
    store.take_events(&mut events);
    let huge_slots = ((PAGE_BUDGET_BYTES - store.bytes() + store.page(b).unwrap().bytes) / (17 * 18 * 16)) as u32;
    let e = store.create(2, (9, 9), huge_slots, 0).unwrap();
    events.clear();
    store.take_events(&mut events);
    assert_eq!(events, vec![PageEvent::Drop(b), PageEvent::New(e)]);
    assert!(store.create(2, (9, 9), huge_slots, 0).is_none());
}

#[test]
fn a_move_keeps_the_source_layout() {
    let mut store = store();
    let from = store.create(2, (0, 0), 3, 6).unwrap();
    let page = store.page_mut(from).unwrap();
    let a = page.place(None, 2);
    let _b = page.place(None, 3);
    page.vacate(a);
    let to = store.create(2, (0, 0), 6, 12).unwrap();
    store.copy_layout(from, to);
    let moved = store.page(to).unwrap();
    // The new slots (5, 4, 3) first in the list, then the source's free ones (2, 0): slot 0 next.
    assert_eq!(moved.free, vec![5, 4, 3, 2, 0]);
    assert_eq!(moved.runs, vec![(0, 2), (5, 7)]);
}

#[test]
fn terrain_09_bounds_the_pooled_pages() {
    let mut store = store();
    let page_bytes = store.page_bytes(2, 32, 96);
    store.set_policy(PoolPolicy {
        kept_per_size: FREE_PAGES_KEPT,
        kept_bytes: Some(3 * page_bytes),
    });
    let mut ids = Vec::new();
    for x in 0..12 {
        // Two sizes, six pages each: without the bound the pool would keep eight.
        ids.push(store.create(2, (x, 0), 32 + 32 * (x as u32 % 2), 96).unwrap());
    }
    for id in ids {
        store.release(id);
        assert!(store.pooled_bytes() <= 3 * page_bytes + store.page_bytes(2, 64, 96));
    }
    assert!(store.pooled_bytes() <= 3 * page_bytes);
    assert_eq!(store.bytes(), store.pooled_bytes(), "a freed page leaves the budget");
    // Without the bound, four a size.
    let mut store = self::store();
    let ids: Vec<u32> = (0..12).map(|x| store.create(2, (x, 0), 32, 96).unwrap()).collect();
    for id in ids {
        store.release(id);
    }
    assert_eq!(store.pooled_bytes(), 4 * page_bytes);
}

// Replays the GDScript allocator's calls recorded in a renderer trace (terrain_graded_trace.gd:
// its `= level`, `= place` and `= vacate` notes and its `> page_*` outputs, in order) against the
// Rust allocator: every page id, event, placed copy and refusal must be the GDScript's. Returns
// the operations replayed.
fn replay_allocator(trace: &str) -> usize {
    let mut store = PageStore::default();
    let mut pending = Vec::new();
    let mut events = Vec::new();
    let mut replayed = 0;
    let int = |word: &str| word.parse::<i64>().unwrap_or_else(|_| panic!("not an integer: {word}"));
    let block = |word: &str| {
        let (x, z) = word.split_once(',').expect("a block");
        (int(x) as i32, int(z) as i32)
    };
    for (number, line) in trace.lines().enumerate() {
        let words: Vec<&str> = line.split(' ').collect();
        let at = || format!("trace line {}: {line}", number + 1);
        let mut check = |store: &mut PageStore, pending: &mut Vec<PageEvent>| {
            events.clear();
            store.take_events(&mut events);
            assert_eq!(events, *pending, "{}", at());
            pending.clear();
        };
        match (words[0], words.get(1).copied().unwrap_or("")) {
            ("<", "setup") => store = PageStore::default(),
            ("=", "level") => {
                assert_eq!(int(words[2]) as usize, store.levels(), "{}", at());
                store.add_level(int(words[3]) as u64);
            }
            (">", "page_drop") => pending.push(PageEvent::Drop(int(words[2]) as u32)),
            (">", "page_new" | "page_reuse") => {
                let id = int(words[2]) as u32;
                pending.push(if words[1] == "page_new" { PageEvent::New(id) } else { PageEvent::Reuse(id) });
                let made = store.create(int(words[3]) as u8, block(words[4]), int(words[5]) as u32, int(words[6]) as u32);
                assert_eq!(made, Some(id), "{}", at());
                check(&mut store, &mut pending);
            }
            (">", "page_refused") => {
                pending.push(PageEvent::Refused);
                let made = store.create(int(words[2]) as u8, block(words[3]), int(words[4]) as u32, int(words[5]) as u32);
                assert_eq!(made, None, "{}", at());
                check(&mut store, &mut pending);
            }
            (">", "page_release") => {
                let id = int(words[2]) as u32;
                pending.push(PageEvent::Release { id, kept: words[3] == "1" });
                store.release(id);
                check(&mut store, &mut pending);
            }
            (">", "page_copy") => {
                let (from, to) = (int(words[2]) as u32, int(words[3]) as u32);
                pending.push(PageEvent::Copy { from, to });
                store.copy_layout(from, to);
                check(&mut store, &mut pending);
            }
            ("=", "place") => {
                let page = store.page_mut(int(words[2]) as u32).unwrap_or_else(|| panic!("{}", at()));
                let held = (int(words[3]) >= 0).then(|| Copy {
                    slot: int(words[3]) as u32,
                    first: int(words[4]) as u32,
                    layers: int(words[5]) as u32,
                });
                let layers = int(words[6]) as u32;
                assert!(page.can_place(held, layers), "{}", at());
                let copy = page.place(held, layers);
                let want = (int(words[7]) as u32, int(words[8]) as u32, int(words[9]) as u32);
                assert_eq!((copy.slot, copy.first, copy.layers), want, "{}", at());
            }
            ("=", "vacate") => {
                let page = store.page_mut(int(words[2]) as u32).unwrap_or_else(|| panic!("{}", at()));
                page.vacate(Copy {
                    slot: int(words[3]) as u32,
                    first: int(words[4]) as u32,
                    layers: int(words[5]) as u32,
                });
            }
            _ => continue,
        }
        replayed += 1;
    }
    assert!(pending.is_empty(), "a page drop with no creation after it");
    replayed
}

#[test]
fn the_allocator_replays_the_gdscript_allocator() {
    // Extracted from the control's traces (`RENDER-32-I` capture): the allocator's lines only.
    let replayed = replay_allocator(include_str!("alloc_golden.txt"));
    assert!(replayed > 1000, "{replayed} operations");
}

// The full traces: METRUM_GRADED_TRACES=<trace>[;<trace>...] (artefacts, not in the repository).
#[test]
#[ignore = "reads recorded traces named by METRUM_GRADED_TRACES"]
fn the_allocator_replays_recorded_traces() {
    let paths = std::env::var("METRUM_GRADED_TRACES").expect("METRUM_GRADED_TRACES");
    for path in paths.split(';').filter(|path| !path.is_empty()) {
        let trace = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let replayed = replay_allocator(&trace);
        eprintln!("{path}: {replayed} allocator operations replayed");
    }
}
