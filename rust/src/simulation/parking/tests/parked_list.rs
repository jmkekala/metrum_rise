// SPDX-License-Identifier: GPL-2.0-only

//! `PARK-02` parked chunks: the per-chunk parked list and the pooled rebuild that reads it,
//! against the full scan and the fresh buffers they replaced.

use super::render::{ParkedChunk, flat_pose};
use super::*;
use crate::simulation::core::stable_hash64;
use crate::simulation::zoning::profiles::ZoneDensity;
use std::collections::BTreeMap;

type Rows = BTreeMap<u8, (Vec<u32>, Vec<u64>)>;

// The rebuild before the list, kept as the reference: every bay of the chunk, in handle order,
// into fresh maps.
fn scanned(s: &ParkingSystem, key: (i32, i32)) -> Rows {
    let mut parked: Vec<_> = s.chunks[&key]
        .places
        .iter()
        .filter_map(|&handle| {
            let p = s.place_handles.resolve(handle).expect("render bay");
            let v = s.vehicle_handles.resolve(s.places.occupant[p])?;
            (!s.vehicles.moving[v]).then_some((handle, p, v))
        })
        .collect();
    parked.sort_unstable();
    let mut rows = Rows::new();
    for (_, p, v) in parked {
        let model = s.vehicles.model[v];
        let pose = flat_pose(
            model,
            s.places.source[p],
            s.places.position[p],
            s.places.heading[p],
        );
        let row = rows.entry(model * 10 + s.vehicles.colour[v]).or_default();
        row.0.extend(pose.map(f32::to_bits));
        row.1.push(s.places.occupant[p].to_bits() | (1_u64 << 62));
    }
    rows
}

fn rows(chunk: &ParkedChunk) -> Rows {
    chunk
        .buckets()
        .map(|(bucket, poses, ids)| {
            let poses = poses.iter().map(|f| f.to_bits()).collect();
            (bucket, (poses, ids.to_vec()))
        })
        .collect()
}

// Every published chunk, bit for bit, against what the old rebuild gives for the same state.
fn assert_published_equals_a_scan(s: &ParkingSystem) {
    for (key, chunk) in s.publications(true) {
        assert_eq!(rows(&chunk), scanned(s, key), "chunk {key:?}");
    }
}

fn bay_at(x: f32, z: f32, n: u64) -> Bay {
    Bay {
        identity: [n, 0],
        position: [x, 0.0, z],
        path: [[x, z]; 4],
        heading: 0.25,
        width: 2.5,
        length: 6.0,
    }
}

fn kerb(edge: u32) -> PlaceSource {
    PlaceSource::Kerb { edge, right: false }
}

// A deterministic draw, one hash per call.
struct Draw(u64);

impl Draw {
    fn below(&mut self, n: usize) -> usize {
        self.0 += 1;
        (stable_hash64(self.0) % n as u64) as usize
    }
}

fn car_of(s: &ParkingSystem, draw: &mut Draw) -> Option<Handle<Vehicle>> {
    (!s.vehicles.is_empty()).then(|| s.vehicle_handles.handle(draw.below(s.vehicles.len())))
}

fn place_of(s: &ParkingSystem, draw: &mut Draw) -> Option<Handle<Place>> {
    (!s.places.is_empty()).then(|| s.place_handles.handle(draw.below(s.places.len())))
}

// One source's row of up to eight bays in chunk `edge % 3`; `mask` keeps some, as an edit does.
fn row_of_bays(edge: u32, mask: usize) -> Vec<Bay> {
    let x = (edge % 3) as f32 * 512.0 + 20.0;
    (0..8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| bay_at(x + i as f32 * 7.0, edge as f32 * 4.0, u64::from(edge)))
        .collect()
}

#[test]
fn the_parked_list_and_the_buffers_match_a_scan_through_custody_edits_and_loads() {
    let mut s = ParkingSystem::default();
    let mut draw = Draw(0);
    for edge in 0..9 {
        s.replace_source(kerb(edge), &row_of_bays(edge, 0xff));
    }
    let mut next_owner = 0;
    for _ in 0..30 {
        s.assign_household(next_owner, ZoneDensity::Low, [100.0, 20.0]);
        next_owner += 1;
    }
    let agent = crate::simulation::economy::agents::AgentHandle::from_parts(1, 0);
    let (mut rebuilds, mut loads, mut most_parked) = (0, 0, 0);
    for step in 0..8000 {
        let (car, place) = (car_of(&s, &mut draw), place_of(&s, &mut draw));
        match draw.below(16) {
            0..=4 => {
                if let (Some(car), Some(place)) = (car, place) {
                    s.occupy(car, place);
                }
            }
            5 | 6 => {
                if let Some(car) = car {
                    s.release(car);
                }
            }
            7 | 8 => {
                if let Some(car) = car {
                    s.set_moving(car, draw.below(2) == 0);
                }
            }
            // A whole outing's custody: borrowed, off the bay, on another.
            9 | 10 => {
                if let (Some(car), Some(place)) = (car, place)
                    && s.reserve_trip(car, agent, place)
                {
                    s.set_moving(car, true);
                    s.release(car);
                    s.occupy(car, place);
                    s.set_moving(car, false);
                    if draw.below(2) == 0 {
                        s.finish_outing(car, agent);
                    }
                }
            }
            // An edit, its undo and everything between: bays go, come back, or move on.
            11 | 12 => {
                let edge = draw.below(9) as u32;
                s.replace_source(kerb(edge), &row_of_bays(edge, draw.below(256)));
            }
            13 => {
                if let Some(place) = place {
                    s.remove_place(place);
                }
            }
            14 => {
                s.remove_household_car(next_owner - 1 - draw.below(5) as u64);
            }
            _ => {
                s.assign_household(next_owner, ZoneDensity::Low, [600.0, 20.0]);
                next_owner += 1;
            }
        }
        s.check_parked_lists().unwrap();
        if step % 5 == 0 {
            s.build_dirty_chunks();
            rebuilds += 1;
            assert_published_equals_a_scan(&s);
            let parked = s.chunks.values().map(|chunk| chunk.parked.len()).sum();
            most_parked = most_parked.max(parked);
        }
        if step % 700 == 699 {
            let json = s.save_json(|source| source).unwrap();
            s = ParkingSystem::load_json(&json).unwrap();
            s.check_parked_lists().unwrap();
            s.build_dirty_chunks();
            assert_published_equals_a_scan(&s);
            assert_eq!(s.save_json(|source| source).unwrap(), json);
            loads += 1;
        }
    }
    assert_eq!((rebuilds, loads), (1600, 11));
    assert!(
        most_parked >= 10,
        "the churn parked {most_parked} cars at most"
    );
}

#[test]
fn a_rebuild_reads_the_parked_list_and_no_empty_bay() {
    let mut s = ParkingSystem::default();
    let bays: Vec<_> = (0..2000)
        .map(|i| s.add_place(source(i), bay(i as f32 * 0.2, 4.0)))
        .collect();
    let cars: Vec<_> = (0..3)
        .map(|owner| s.assign_household(owner, ZoneDensity::Low, [0.0, 4.0]))
        .collect();
    for (&car, place) in cars.iter().zip([10, 900, 1500]) {
        assert!(s.occupy(car, bays[place]));
    }
    s.build_dirty_chunks();
    let want = scanned(&s, (0, 0));
    assert_eq!(want.values().map(|row| row.1.len()).sum::<usize>(), 3);
    s.set_moving(cars[0], true);
    s.set_moving(cars[0], false);
    let mut jobs = s.take_dirty_jobs();
    // The chunk's bay list is the way to its empty bays, so a rebuild goes without it.
    let chunk = s.chunks.get_mut(&(0, 0)).unwrap();
    let places = std::mem::take(&mut chunk.places);
    assert_eq!((places.len(), chunk.parked.len()), (2000, 3));
    s.solve_dirty_chunks(&mut jobs, flat_pose);
    s.chunks.get_mut(&(0, 0)).unwrap().places = places;
    s.install_dirty_chunks(jobs);
    assert_eq!(rows(&s.publications(true)[0].1), want);
}

// Two chunks of 40 cars on 80 bays; a car parks on a bay another model left, as at rush hour.
fn churn_fixture() -> (ParkingSystem, Vec<Handle<Vehicle>>) {
    let mut s = ParkingSystem::default();
    for edge in (0..30).filter(|edge| edge % 3 != 2) {
        s.replace_source(kerb(edge), &row_of_bays(edge, 0xff));
    }
    let places: Vec<_> = (0..s.places.len())
        .map(|p| s.place_handles.handle(p))
        .collect();
    let cars: Vec<_> = (0..80)
        .map(|owner| s.assign_household(owner, ZoneDensity::Low, [0.0, 0.0]))
        .collect();
    for (&car, &place) in cars.iter().zip(places.iter().step_by(2)) {
        assert!(s.occupy(car, place));
    }
    (s, cars)
}

// One departure and arrival inside a chunk: the car count of each chunk stays what it was.
fn move_a_car(s: &mut ParkingSystem, draw: &mut Draw, cars: &[Handle<Vehicle>]) {
    let car = cars[draw.below(cars.len())];
    let v = s.vehicle_handles.resolve(car).unwrap();
    let from = s.place_handles.resolve(s.vehicles.place[v]).unwrap();
    let key = chunk_at([s.places.position[from][0], s.places.position[from][2]]);
    let chunk = &s.chunks[&key];
    let free = (0..chunk.places.len())
        .map(|_| chunk.places[draw.below(chunk.places.len())])
        .find(|&place| {
            let p = s.place_handles.resolve(place).unwrap();
            s.places.occupant[p].is_none()
        })
        .expect("half the bays are free");
    s.set_moving(car, true);
    s.release(car);
    assert!(s.occupy(car, free));
    s.set_moving(car, false);
}

// The sim thread's two snapshots: the one Godot reads, and the one being rebuilt.
#[derive(Default)]
struct Snapshots {
    shared: ParkedPublication,
    recycled: ParkedPublication,
}

impl Snapshots {
    fn publish(&mut self, s: &mut ParkingSystem, renderer: bool) {
        s.release_snapshot(&mut self.recycled);
        s.take_renderer_answers();
        s.build_dirty_chunks();
        self.recycled = s.publication();
        std::mem::swap(&mut self.shared, &mut self.recycled);
        if renderer {
            self.shared.acknowledge(self.shared.serial);
        }
    }
}

// Every buffer a rebuild could make or grow, and how many it made.
fn footprint(s: &ParkingSystem) -> [usize; 2] {
    let of = |chunk: &ParkedChunk| chunk.capacities();
    let chunks: usize = s
        .chunks
        .values()
        .map(|chunk| chunk.parked.capacity() + chunk.spare.as_deref().map_or(0, of))
        .sum();
    let published: usize = s.published.values().map(|chunk| of(chunk)).sum();
    let jobs: usize = s.jobs.iter().map(|job| job.solved_capacity()).sum();
    let lists = s.publication.updates.capacity() + s.spare_updates.capacity();
    let queues = s.pending.capacity() + s.jobs.capacity() + s.dirty.capacity();
    [
        s.buffer_creations(),
        chunks + published + jobs + lists + queues,
    ]
}

#[test]
fn custody_churn_at_a_stable_car_count_makes_no_buffer_after_warm_up() {
    for renderer in [true, false] {
        let (mut s, cars) = churn_fixture();
        let mut snapshots = Snapshots::default();
        let mut draw = Draw(7);
        let mut churn = |s: &mut ParkingSystem, frames: usize, moves: usize| {
            for _ in 0..frames {
                for _ in 0..moves {
                    move_a_car(s, &mut draw, &cars);
                }
                snapshots.publish(s, renderer);
                assert_published_equals_a_scan(s);
            }
        };
        churn(&mut s, 50, 6);
        let warm = footprint(&s);
        assert!(warm[0] > 0);
        churn(&mut s, 2000, 6);
        assert_eq!(footprint(&s), warm, "renderer {renderer}");
        assert!(s.jobs.iter().any(|job| job.solved_capacity() > 0));
        // Nothing answers headless, and still a chunk is listed once.
        let listed = if renderer { 0..=2 } else { 2..=2 };
        assert!(listed.contains(&s.pending.len()), "{}", s.pending.len());
        assert!(s.publication.updates.len() <= 2);
    }
}

#[test]
fn a_chunk_someone_still_holds_is_never_written() {
    let (mut s, cars) = churn_fixture();
    let mut snapshots = Snapshots::default();
    let mut draw = Draw(11);
    snapshots.publish(&mut s, true);
    for _ in 0..4 {
        move_a_car(&mut s, &mut draw, &cars);
        snapshots.publish(&mut s, true);
    }
    let held: Vec<_> = s
        .publications(true)
        .into_iter()
        .map(|(_, chunk)| chunk)
        .collect();
    let kept_list = snapshots.shared.clone();
    let before: Vec<_> = held.iter().map(|chunk| rows(chunk)).collect();
    let listed: Vec<_> = kept_list.updates.iter().map(|u| rows(&u.chunk)).collect();
    let made = s.buffer_creations();
    for _ in 0..50 {
        move_a_car(&mut s, &mut draw, &cars);
        snapshots.publish(&mut s, true);
    }
    assert_eq!(
        held.iter().map(|chunk| rows(chunk)).collect::<Vec<_>>(),
        before
    );
    let now: Vec<_> = kept_list.updates.iter().map(|u| rows(&u.chunk)).collect();
    assert_eq!(now, listed);
    assert!(
        s.buffer_creations() > made,
        "a held buffer is replaced, not reused"
    );
    assert_published_equals_a_scan(&s);
}
