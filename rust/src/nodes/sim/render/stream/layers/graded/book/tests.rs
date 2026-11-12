// SPDX-License-Identifier: GPL-2.0-only

//! The book's result path and edit batch without the engine (`CODE-30`, the streaming audit's
//! finding 5): a [`Host`] that records the renderer's callbacks and scripts the polls and the
//! budget's clock drives [`Run`] as `terrain_graded_tiles.gd` does through [`ScriptHost`]. The
//! book has no cell objects here, so the cells' runs reach the host with the other effects.

use super::edits::{Edit, EditTile};
use super::results::{Host, Run};
use super::*;
use std::collections::VecDeque;

// The patch side and the world: 2 x 2 patches of 510 m from (-510, -510), blocks of 2 patches.
const SPAN: f32 = 510.0;
// Four tiles of patch (0, 0), all in GL2's band from the camera below.
const TILES: [Tile; 4] = [
    Tile { x: -4, z: -4 },
    Tile { x: -3, z: -4 },
    Tile { x: -4, z: -3 },
    Tile { x: -3, z: -3 },
];

// The renderer as the tests see it: the book, the polls it will hand out, the budget's clock
// (advanced by `tick` at each reading), and what was called back.
struct Recorder {
    book: GradedTileBook,
    polls: VecDeque<Vec<Polled>>,
    clock: i64,
    tick: i64,
    // Effect kinds the renderer applied (page arrays, cancels, draw states), in order.
    applied: Vec<i64>,
    // `release_preview` calls.
    released: Vec<(i64, i64)>,
}

impl Host for Recorder {
    type Book<'a> = &'a mut GradedTileBook;

    fn book(&mut self) -> Self::Book<'_> {
        &mut self.book
    }

    // `_apply_effects` without its Godot objects: the runs of writes and cells go back to the
    // book (which counts the writes and has no cell objects), the rest is recorded.
    fn apply_effects(&mut self) {
        self.book.encode_effects();
        let effects = std::mem::take(&mut self.book.encoded);
        for effect in effects.as_chunks::<EFFECT_FIELDS>().0 {
            match effect[0] {
                47 => self.book.apply_cells(effect[1], effect[2]),
                48 => self.book.apply_writes(effect[1], effect[2]),
                kind => self.applied.push(kind),
            }
        }
        self.book.encoded = effects;
    }

    fn release_preview(&mut self, first: i64, last: i64) {
        self.released.push((first, last));
        Run::unbudgeted(&mut *self).release_epochs(first, last);
    }

    fn poll(&mut self, polled: &mut Vec<Polled>, _spent: &mut Vec<Vec<u8>>) -> i64 {
        polled.extend(self.polls.pop_front().unwrap_or_default());
        0
    }

    fn now_us(&mut self) -> i64 {
        self.clock += self.tick;
        self.clock
    }
}

// The recorder borrowed is a host too (a run inside a callback).
impl Host for &mut Recorder {
    type Book<'a>
        = &'a mut GradedTileBook
    where
        Self: 'a;

    fn book(&mut self) -> Self::Book<'_> {
        &mut self.book
    }

    fn apply_effects(&mut self) {
        (**self).apply_effects();
    }

    fn release_preview(&mut self, first: i64, last: i64) {
        (**self).release_preview(first, last);
    }

    fn poll(&mut self, polled: &mut Vec<Polled>, spent: &mut Vec<Vec<u8>>) -> i64 {
        (**self).poll(polled, spent)
    }

    fn now_us(&mut self) -> i64 {
        (**self).now_us()
    }
}

fn plain_book() -> GradedTileBook {
    GradedTileBook {
        state: Graded::default(),
        scratch: Vec::new(),
        tiles: Vec::new(),
        flat: Vec::new(),
        results: Vec::new(),
        considered: 0,
        before: Shown::default(),
        encoded: Vec::new(),
        trace: None,
        objects: None,
        backend: None,
        sync: MaterialSync::default(),
        cell_effects: Vec::new(),
        cell_counts: [0; CELL_COUNTERS],
        page_textures: Default::default(),
        page_shapes: Default::default(),
        staged: None,
        writes: Vec::new(),
        write_bake: None,
        unwritten: Vec::new(),
        upload_counts: [0; 3],
        taken: results::Results::default(),
        edits: edits::Edits::default(),
        walls_scratch: Vec::new(),
        patch_news: Default::default(),
        list_requests: Default::default(),
    }
}

// A world with patch (0, 0) resident and drawn by a built set, its list naming `TILES`, every
// tile requested at its level (the hand-over answered "queued") and the effects applied, as the
// renderer's first frames leave it.
fn world() -> Recorder {
    let mut book = plain_book();
    let shape = Shape {
        world_min: [-SPAN, -SPAN],
        patch_span: SPAN,
        patch_grid: [2, 2],
        block_patches: 2,
        layer_limit: 2048,
    };
    let width = u64::from(ATLAS_WIDTH);
    book.state.setup(
        shape,
        &[
            (131, 64, 24 * width),
            (67, 192, 18 * width),
            (35, 96, 9 * width),
            (35, 256, 9 * width),
        ],
    );
    book.state.pages.set_policy(PoolPolicy::unbounded());
    book.cells_setup(81, 4);
    let key = Vector2i::new(0, 0);
    let rect = Rect2::new(Vector2::new(-SPAN, -SPAN), Vector2::new(SPAN, SPAN));
    book.set_patch(key, true, rect, 1, 0);
    book.patch_changed(key, true, rect, false);
    book.move_camera(Vector3::new(-200.0, 300.0, -200.0));
    book.list_cells(key, true);
    book.state.take_list((0, 0), 0, TILES.to_vec());
    while book.state.apply_count() {}
    // The set's build, whole (the renderer's `_advance_builds` without a budget).
    loop {
        let next = book.next_build();
        if next.z < 0 {
            break;
        }
        let key = Vector2i::new(next.x, next.y);
        let set = book.begin_build(key);
        book.state.settle();
        book.state.build_tiles(set as u32, None);
        book.build_finished(key, true);
    }
    let mut recorder = Recorder {
        book,
        polls: VecDeque::new(),
        clock: 0,
        tick: 0,
        applied: Vec::new(),
        released: Vec::new(),
    };
    recorder.apply_effects();
    recorder.request_all();
    recorder
}

impl Recorder {
    // The hand-over, every request queued; the effects applied. Returns the requests, flat
    // `(x, z, code, step, token)`.
    fn request_all(&mut self) -> Vec<i64> {
        self.book.request_tiles();
        assert!(self.book.state.requests.stale_cancels.is_empty());
        let flat = self.book.flat.clone();
        self.book.hand_over_with(|_, requests, answers| {
            answers.extend(std::iter::repeat_n(1, requests.len() / 5))
        });
        self.apply_effects();
        flat
    }

    // The requested level and token of `tile`.
    fn request(&self, tile: Tile) -> (u8, u8, i64) {
        let (key, token) = self
            .book
            .state
            .requests
            .requested
            .iter()
            .find(|((requested, _, _), _)| *requested == tile)
            .map(|(key, token)| (*key, *token))
            .expect("the tile is requested");
        (key.1, key.2, token)
    }

    // A bake of `tile` at its requested level and token (`behind`: flagged behind the road).
    fn baked(&self, tile: Tile, behind: bool) -> Baked {
        let (code, step, token) = self.request(tile);
        bake(tile, (code, step), token, behind)
    }

    // A frame's result path from `start_us`.
    fn frame(&mut self, start_us: i64) {
        Run::budgeted(&mut *self, start_us).results();
    }

    fn installed(&self) -> Vec<Tile> {
        self.book.state.installed.keys().copied().collect()
    }

    fn token(&self, tile: Tile) -> Option<i64> {
        self.book
            .state
            .installed
            .get(&tile)
            .map(|entry| entry.token)
    }
}

// A bake's bytes: an empty slot layer, one atlas layer of rows, no wall faces.
fn bake(tile: Tile, level: (u8, u8), token: i64, behind: bool) -> Baked {
    let rows = vec![0; uploads::ATLAS_LAYER_BYTES];
    Baked::owned(
        tile,
        (i64::from(level.0), i64::from(level.1)),
        token,
        behind,
        [Vec::new(), rows, Vec::new()],
    )
}

#[test]
fn the_fixture_requests_every_listed_tile_at_gl2() {
    let world = world();
    for tile in TILES {
        assert_eq!(world.request(tile).0, 2, "tile {tile:?} at GL2");
    }
    assert!(world.installed().is_empty());
}

#[test]
fn a_budget_that_runs_out_mid_poll_defers_and_resumes_in_order() {
    let mut world = world();
    let results: Vec<Polled> = TILES
        .iter()
        .map(|&tile| Polled::Tile(world.baked(tile, false)))
        .collect();
    world.polls.push_back(results);
    // The first result is taken unconditionally; each later one checks the budget first: 600 us
    // a reading, so the second fits (608 us with its placed tile charged) and the third does not.
    world.tick = 600;
    world.frame(0);
    assert_eq!(world.installed(), TILES[..2]);
    assert_eq!(world.book.taken_deferred(), 2);
    assert_eq!(world.book.result_counts()[0], 2, "two results deferred");
    // The next frame takes the deferred results first, in arrival order, and polls nothing new
    // once its own budget is spent.
    let start = world.clock;
    world.frame(start);
    assert_eq!(world.installed(), TILES);
    assert_eq!(world.book.taken_deferred(), 0);
    assert!(
        world.polls.is_empty(),
        "the deferred frame polled before its budget ran out"
    );
    // Every tile installed at the level and token it was requested with, and none is still
    // requested.
    for tile in TILES {
        assert!(world.token(tile).is_some());
    }
    assert!(world.book.state.requests.requested.len() == 0);
}

#[test]
fn a_stale_result_is_dropped() {
    let mut world = world();
    let (code, step, token) = world.request(TILES[0]);
    world.polls.push_back(vec![Polled::Tile(bake(
        TILES[0],
        (code, step),
        token + 1,
        false,
    ))]);
    world.frame(0);
    assert!(world.installed().is_empty());
    assert_eq!(
        world.request(TILES[0]).2,
        token,
        "still requested at its token"
    );
}

#[test]
fn a_flagged_tile_waits_for_its_sets_variant() {
    let mut world = world();
    world.polls.push_back(vec![
        Polled::Tile(world.baked(TILES[0], true)),
        Polled::Tile(world.baked(TILES[1], false)),
    ]);
    world.frame(0);
    // The flagged tile waits (still requested); the other installs.
    assert_eq!(world.installed(), [TILES[1]]);
    assert!(world.book.results_waiting());
    assert_eq!(world.book.result_counts()[1], 1, "one variant wait");
    assert!(
        world
            .book
            .state
            .requests
            .requested
            .keys()
            .any(|key| key.0 == TILES[0])
    );
    // Nothing installs while the set lacks the variant.
    Run::unbudgeted(&mut world).variant_waits();
    assert_eq!(world.installed(), [TILES[1]]);
    // The build path gives the set the variant; the wait installs.
    assert_eq!(world.book.add_next_variant(), 1);
    world.apply_effects();
    Run::unbudgeted(&mut world).variant_waits();
    assert_eq!(world.installed(), [TILES[1], TILES[0]]);
    assert!(!world.book.results_waiting());
    assert!(
        world
            .book
            .state
            .installed
            .get(&TILES[0])
            .is_some_and(|entry| entry.behind)
    );
}

// Installs every tile of the fixture.
fn installed_world() -> Recorder {
    let mut world = world();
    let results: Vec<Polled> = TILES
        .iter()
        .map(|&tile| Polled::Tile(world.baked(tile, false)))
        .collect();
    world.polls.push_back(results);
    world.frame(0);
    assert_eq!(world.installed(), TILES);
    world
}

// An edit of `serial` dirtying `tiles` at GL2.
fn edit(serial: i64, tiles: &[Tile]) -> Edit {
    Edit {
        serial,
        all: false,
        levels: vec![(2, 0, tiles.to_vec())],
        preview_epoch: -1,
        staged: Vec::new(),
    }
}

#[test]
fn an_edit_batch_installs_whole_or_not_at_all() {
    let mut world = installed_world();
    let tokens: Vec<i64> = TILES
        .iter()
        .map(|&tile| world.token(tile).unwrap())
        .collect();
    world
        .polls
        .push_back(vec![Polled::Edit(edit(5, &TILES[..2]))]);
    world.frame(0);
    assert_eq!(world.book.edit_batch_len(), 2);
    // The tiles rebake with the edit's serial as their token.
    world.request_all();
    assert_eq!(world.request(TILES[0]).2, 5);
    // One result in: the batch waits, the installed tiles keep drawing as they were.
    world
        .polls
        .push_back(vec![Polled::Tile(world.baked(TILES[0], false))]);
    world.frame(0);
    assert_eq!(Run::unbudgeted(&mut world).install_edit_batch(), 0);
    assert_eq!(world.token(TILES[0]), Some(tokens[0]));
    assert_eq!(
        world.book.installed_edit_serial(),
        0,
        "T61: the batch holds the serial back"
    );
    // The other result in: both install in one call.
    world
        .polls
        .push_back(vec![Polled::Tile(world.baked(TILES[1], false))]);
    world.frame(0);
    assert_eq!(Run::unbudgeted(&mut world).install_edit_batch(), 2);
    assert_eq!(world.token(TILES[0]), Some(5));
    assert_eq!(world.token(TILES[1]), Some(5));
    assert_eq!(
        world.token(TILES[2]),
        Some(tokens[2]),
        "an undirtied tile is untouched"
    );
    assert_eq!(world.book.last_batch_tiles(), &TILES[..2]);
    assert_eq!(world.book.installed_edit_serial(), 5);
    assert_eq!(world.book.edit_batch_len(), 0);
}

#[test]
fn a_refused_tile_leaves_the_batch_and_the_rest_installs() {
    let mut world = installed_world();
    world
        .polls
        .push_back(vec![Polled::Edit(edit(5, &TILES[..2]))]);
    world.frame(0);
    world.request_all();
    world
        .polls
        .push_back(vec![Polled::Tile(world.baked(TILES[0], false))]);
    world.frame(0);
    // `_failed` of a tile refused by contract: the request dropped, the tile out of the batch.
    let (code, step, token) = world.request(TILES[1]);
    let tile = Vector2i::new(TILES[1].x, TILES[1].z);
    world
        .book
        .failed_tile(tile, i64::from(code), i64::from(step), token, false);
    world.book.edit_unstage(tile);
    world.book.tile_refused(tile);
    world.apply_effects();
    assert_eq!(Run::unbudgeted(&mut world).install_edit_batch(), 1);
    assert_eq!(world.token(TILES[0]), Some(5));
}

#[test]
fn a_preview_frame_draws_over_the_installed_tile_until_released() {
    let mut world = installed_world();
    let installed = world.token(TILES[0]);
    // Pose epoch 3: tile 0 takes slots of its own (the token is the epoch).
    Run::unbudgeted(&mut world).preview_frame(&[], [bake(TILES[0], (2, 0), 3, false)]);
    assert_eq!(
        world
            .book
            .state
            .preview
            .get(&TILES[0])
            .map(|entry| entry.token),
        Some(3)
    );
    assert_eq!(
        world.token(TILES[0]),
        installed,
        "the installed tile is kept under it"
    );
    // Pose epoch 4 moves the preview: tile 0 removed, tile 1 shown.
    Run::unbudgeted(&mut world).preview_frame(
        &[TILES[0].x, TILES[0].z],
        [bake(TILES[1], (2, 0), 4, false)],
    );
    assert!(!world.book.state.preview.contains_key(&TILES[0]));
    assert!(world.book.state.preview.contains_key(&TILES[1]));
    // A cancelled preview drops its epoch.
    Run::unbudgeted(&mut world).release_epochs(4, 4);
    assert!(world.book.state.preview.len() == 0);
}

#[test]
fn a_flagged_preview_tile_waits_for_the_variant() {
    let mut world = installed_world();
    Run::unbudgeted(&mut world).preview_frame(&[], [bake(TILES[2], (2, 0), 3, true)]);
    assert!(
        world.book.state.preview.len() == 0,
        "T27 (d): the preview lags"
    );
    assert!(world.book.has_preview_waits());
    assert_eq!(world.book.add_next_variant(), 1);
    world.apply_effects();
    Run::unbudgeted(&mut world).preview_waits();
    assert!(world.book.state.preview.contains_key(&TILES[2]));
    assert!(!world.book.has_preview_waits());
}

#[test]
fn a_commit_adopts_the_preview_in_the_frame_its_batch_installs() {
    let mut world = installed_world();
    let installed = world.token(TILES[0]);
    Run::unbudgeted(&mut world).preview_frame(&[], [bake(TILES[0], (2, 0), 3, false)]);
    // The commit (serial 7) dirties tile 0 and carries its own tile, bitwise the preview's
    // payload; tile 1 rebakes.
    let mut commit = edit(7, &TILES[..2]);
    commit.preview_epoch = 3;
    commit.staged.push(EditTile {
        baked: bake(TILES[0], (2, 0), 0, false),
        graded: true,
        adopted: true,
    });
    world.polls.push_back(vec![Polled::Edit(commit)]);
    world.frame(0);
    world.request_all();
    // T60: until the batch installs the preview keeps drawing, the installed tile under it.
    assert_eq!(Run::unbudgeted(&mut world).install_edit_batch(), 0);
    assert!(world.book.state.preview.contains_key(&TILES[0]));
    assert_eq!(world.token(TILES[0]), installed);
    assert_eq!(world.book.installed_edit_serial(), 0);
    // Tile 1's result completes the batch: tile 0 adopts the preview's slots, tile 1 uploads,
    // and the preview of the adopted epoch drops in the same call.
    world
        .polls
        .push_back(vec![Polled::Tile(world.baked(TILES[1], false))]);
    world.frame(0);
    assert_eq!(Run::unbudgeted(&mut world).install_edit_batch(), 2);
    assert!(world.book.state.preview.len() == 0);
    assert_eq!(world.token(TILES[0]), Some(7));
    assert_eq!(world.token(TILES[1]), Some(7));
    assert_eq!(world.book.edit_counts()[1], 1, "one adopted");
    assert_eq!(world.book.installed_edit_serial(), 7);
    assert!(
        world.released.is_empty(),
        "the batch released the preview itself"
    );
}

#[test]
fn a_commit_no_held_tile_takes_releases_its_preview_through_the_renderer() {
    let mut world = installed_world();
    Run::unbudgeted(&mut world).preview_frame(&[], [bake(TILES[0], (2, 0), 3, false)]);
    // A commit whose edit dirties nothing held: its preview ends with it, through the
    // renderer's own entry point.
    let mut commit = edit(8, &[]);
    commit.preview_epoch = 3;
    world.polls.push_back(vec![Polled::Edit(commit)]);
    world.frame(0);
    assert_eq!(Run::unbudgeted(&mut world).install_edit_batch(), 0);
    assert_eq!(world.released, [(0, 3)]);
    assert!(world.book.state.preview.len() == 0);
    assert_eq!(world.book.installed_edit_serial(), 8);
}

#[test]
fn the_cores_entries_are_news_and_follow_steps_and_generations() {
    use crate::nodes::sim::render::stream::layers::terrain::Follow;
    let mut book = plain_book();
    let rect = [-510.0, -510.0, 510.0, 510.0];
    book.follow_terrain([0, 0], Follow::Resident { step: 2, generation: 7 }, rect);
    book.follow_terrain([1, 0], Follow::Resident { step: 1, generation: 3 }, [0.0, -510.0, 510.0, 510.0]);
    // A step change keeps the rectangle and generation; a commit keeps the step.
    book.follow_terrain([0, 0], Follow::Step(4), [0.0; 4]);
    book.follow_generation([0, 0], 8);
    assert_eq!(book.state.resident.get(&(0, 0)), Some(&Patch { rect, step: 4, generation: 8 }));
    // Patches not resident take nothing; a commit of one is news all the same.
    book.follow_terrain([5, 5], Follow::Step(2), rect);
    book.follow_terrain([5, 5], Follow::Left, rect);
    book.follow_generation([5, 5], 1);
    assert!(!book.state.resident.contains_key(&(5, 5)));
    // The news: each changed key once, in the order first changed.
    let news: Vec<_> = book.patch_news.keys().copied().collect();
    assert_eq!(news, [(0, 0), (1, 0), (5, 5)]);
    book.patch_news.clear();
    book.follow_terrain([0, 0], Follow::Left, rect);
    assert!(!book.state.resident.contains_key(&(0, 0)));
    assert_eq!(book.patch_news.keys().copied().collect::<Vec<_>>(), [(0, 0)]);
    // The resident order is the order they came (the renderer's dictionary order).
    book.follow_terrain([0, 0], Follow::Resident { step: 1, generation: 9 }, rect);
    assert_eq!(book.state.resident.keys().copied().collect::<Vec<_>>(), [(1, 0), (0, 0)]);
}
