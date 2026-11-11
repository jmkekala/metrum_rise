// SPDX-License-Identifier: GPL-2.0-only

//! The request queue: nearest first, the in-flight bound, replaced and cancelled requests never
//! started, and no work without a change.

use super::*;

fn request(x: i32, z: i32, level: Level, token: i64) -> TileRequest {
    TileRequest {
        tile: TileKey { x, z },
        level,
        token,
    }
}

// Every pending request starts in the order of its distance from the view (then its key),
// never more than MAX_IN_FLIGHT at once, whatever order they were pushed in.
#[test]
fn requests_start_nearest_first_within_the_in_flight_bound() {
    let mut queue = TileRequestQueue::default();
    queue.set_view([-300.0, 25.0, 700.0]);
    let mut pushed = Vec::new();
    // A scattered order over many chunks, negative keys included.
    for i in 0..1_000i32 {
        let (x, z) = ((i * 37) % 61 - 30, (i * 53) % 47 - 20);
        if queue.push(request(x, z, Level::Gl2, 0)).is_none() {
            pushed.push(TileKey { x, z });
        }
    }
    let view = queue.view;
    pushed.sort_by(|a, b| {
        view.tile_ground_distance_sq(*a)
            .total_cmp(&view.tile_ground_distance_sq(*b))
            .then_with(|| (a.x, a.z).cmp(&(b.x, b.z)))
    });
    let mut issued = Vec::new();
    let mut out = Vec::new();
    while queue.pending_len() > 0 {
        out.clear();
        queue.start(&mut out);
        assert!(queue.in_flight_len() <= MAX_IN_FLIGHT, "the in-flight bound");
        assert!(!out.is_empty(), "free places start pending requests");
        assert!(
            queue.in_flight_len() == MAX_IN_FLIGHT || queue.pending_len() == 0,
            "a start fills every free place"
        );
        issued.extend(out.iter().map(|started| started.tile));
        // Finish a few, as a poll would.
        for started in out.iter().take(7) {
            queue.finish(started.tile, started.level);
        }
    }
    assert_eq!(issued, pushed, "issued nearest first");
}

// A full in-flight count starts nothing; each place a poll frees starts the next request.
#[test]
fn a_freed_place_starts_the_next_request() {
    let mut queue = TileRequestQueue::default();
    queue.set_view([0.0, 0.0, 0.0]);
    for x in 0..(MAX_IN_FLIGHT as i32 + 10) {
        queue.push(request(x, 0, Level::Gl0, 0));
    }
    let mut out = Vec::new();
    queue.start(&mut out);
    assert_eq!(out.len(), MAX_IN_FLIGHT);
    let first = out[0];
    out.clear();
    queue.push(request(-1, 0, Level::Gl0, 0));
    queue.start(&mut out);
    assert!(out.is_empty(), "no place is free");
    queue.finish(first.tile, first.level);
    queue.start(&mut out);
    assert_eq!(out, [request(-1, 0, Level::Gl0, 0)], "the freed place takes the nearest");
}

// A re-levelled tile's newer request replaces its pending one, and a cancelled tile's goes: the
// replaced and cancelled requests are never started. A request whose tile and level is still in
// flight (an edit's newer token) waits until that bake leaves.
#[test]
fn replaced_and_cancelled_requests_never_start() {
    let mut queue = TileRequestQueue::default();
    queue.set_view([0.0, 0.0, 0.0]);
    assert_eq!(queue.push(request(1, 1, Level::Gl2, 0)), None);
    assert_eq!(queue.push(request(1, 1, Level::Gl1, 0)), Some(Level::Gl2));
    queue.push(request(2, 2, Level::Gl0, 0));
    assert_eq!(queue.cancel(TileKey { x: 2, z: 2 }), Some(Level::Gl0));
    assert_eq!(queue.cancel(TileKey { x: 2, z: 2 }), None);
    queue.push(request(3, 3, Level::Gl3 { step: 2 }, 0));
    let mut out = Vec::new();
    queue.start(&mut out);
    assert_eq!(
        out,
        [request(1, 1, Level::Gl1, 0), request(3, 3, Level::Gl3 { step: 2 }, 0)]
    );
    // Tile (1, 1) at GL1 is in flight: its rebake at a newer token waits for it.
    out.clear();
    queue.push(request(1, 1, Level::Gl1, 5));
    queue.start(&mut out);
    assert!(out.is_empty(), "the same tile and level in flight holds the newer request");
    queue.finish(TileKey { x: 1, z: 1 }, Level::Gl1);
    queue.start(&mut out);
    assert_eq!(out, [request(1, 1, Level::Gl1, 5)]);
    queue.clear_pending();
    assert_eq!(queue.pending_len(), 0);
}

// Invariant 3: without a request or a freed place, a start does nothing, however the view
// moves (the order matters only when a bake can start).
#[test]
fn a_start_without_a_change_does_no_work() {
    let mut queue = TileRequestQueue::default();
    queue.set_view([0.0, 0.0, 0.0]);
    for x in 0..200 {
        queue.push(request(x, -x, Level::Gl2, 0));
    }
    let mut out = Vec::new();
    queue.start(&mut out);
    let work = queue.work();
    assert_eq!(work.started, MAX_IN_FLIGHT as u64);
    for frame in 0..10 {
        queue.set_view([f64::from(frame), 0.0, 0.0]);
        queue.start(&mut out);
    }
    assert_eq!(queue.work(), work, "no start walks without a change");
    // Every pending request blocked by its own tile in flight: one walk, then none.
    let mut queue = TileRequestQueue::default();
    queue.push(request(0, 0, Level::Gl0, 0));
    queue.start(&mut out);
    queue.push(request(0, 0, Level::Gl0, 1));
    queue.start(&mut out);
    let work = queue.work();
    queue.start(&mut out);
    queue.start(&mut out);
    assert_eq!(queue.work(), work);
}
