// SPDX-License-Identifier: GPL-2.0-only

//! Thin parking inspector, debug fleet and versioned parked-render bridge. The render path reads
//! the snapshot only: moving and parked cars come from one custody state, and no call here
//! takes the core lock for them (`PARK-02`).

use super::*;
use crate::simulation::core::handles::Handle;
use crate::simulation::parking::stats::DepartureOutcome;

#[godot_api(secondary)]
impl SimulationNode {
    /// Isolated capture/test command: creates one real resident household and an ordinary trip.
    #[func]
    pub fn debug_add_parking_commuter(&mut self, home: i32, target: i32) -> i32 {
        self.lock_core()
            .debug_add_parking_commuter(home as usize, target as usize)
            .map_or(-1, |i| i as i32)
    }

    /// Isolated capture command: schedules an ordinary household move-in from a border.
    #[func]
    pub fn debug_add_parking_move_in(&mut self, home: i32) -> i32 {
        self.lock_core().debug_add_parking_move_in(home as usize).map_or(-1, |i| i as i32)
    }

    /// Isolated fixture clock reset, allowed only before any resident exists.
    #[func]
    pub fn debug_reset_parking_capture_clock(&mut self) {
        self.lock_core().debug_reset_parking_capture_clock();
    }

    /// Schedules capture departures through the existing saved timing wheel.
    #[func]
    pub fn debug_schedule_parking_capture(&mut self, minute: i32, returning: bool) -> i32 {
        self.lock_core()
            .debug_schedule_parking_capture(minute.clamp(0, 1439) as u16, returning) as i32
    }

    /// Advances a deliberately constructed isolated fixture through the ordinary simulation.
    #[func]
    pub fn debug_advance_parking_capture(&mut self, minute: i32) {
        self.lock_core()
            .debug_advance_parking_capture(minute.clamp(0, 1439) as u16);
    }

    /// Explicit debug/test command, O(places), never part of ordinary gameplay initialization.
    #[func]
    pub fn debug_fill_parking(&mut self) -> i32 {
        let mut core = self.lock_core();
        core.publish_pending_building_site_changes();
        core.parking.fill_test_fleet() as i32
    }

    /// Local inspection counts and rejected asset-layout diagnostics; no simulation decisions.
    #[func]
    pub fn get_parking_info(&self) -> VarDictionary {
        let core = self.lock_core();
        let (places, vehicles, stored) = core.parking.counts();
        let mut result = VarDictionary::new();
        result.set("places", places as i64);
        result.set("vehicles", vehicles as i64);
        result.set("stored", stored as i64);
        let breakdown = core.parking.vehicle_breakdown();
        result.set("parked", breakdown.parked as i64);
        result.set("moving", breakdown.moving as i64);
        result.set("borrowed", breakdown.borrowed as i64);
        result.set("reserved", breakdown.reserved as i64);
        let mut departures = VarDictionary::new();
        for outcome in DepartureOutcome::ALL {
            departures.set(outcome.label(), core.parking.departures.count(outcome) as i64);
        }
        result.set("departures", departures);
        let mut phases = VarDictionary::new();
        for trip in &core.agents.parking_trip {
            let name = format!("{:?}", trip.phase);
            let previous = phases
                .get(name.as_str())
                .and_then(|v| v.try_to::<i64>().ok())
                .unwrap_or(0);
            phases.set(name.as_str(), previous + 1);
        }
        result.set("phases", phases);
        let indoor = core
            .agents
            .parking_trip
            .iter()
            .enumerate()
            .filter(|&(i, trip)| {
                !trip.vehicle.is_none()
                    && core.agents.transit_mode[i] == crate::simulation::economy::agents::MODE_CAR
                    && core.agents.transit[i]
                        == crate::simulation::economy::agents::TRANSIT_IN_BUILDING
            })
            .count();
        result.set("resident_cars_in_buildings", indoor as i64);
        result.set(
            "diagnostics",
            PackedStringArray::from_iter(core.parking.diagnostics.iter().flat_map(
                |(&building, issues)| {
                    let id = core
                        .allocator
                        .building_handles
                        .resolve(Handle::from_bits(building))
                        .map_or(-1, |index| index as i64);
                    issues
                        .iter()
                        .map(move |issue| GString::from(format!("building {id}: {issue}").as_str()))
                },
            )),
        );
        result
    }

    /// The parked chunks of the current snapshot the renderer has not installed. `full` asks for
    /// every chunk: a later snapshot lists them, and its payload says `full`. Lock-free.
    #[func]
    pub fn get_parked_car_chunks(&self, full: bool) -> VarDictionary {
        let snapshot = self.snapshot.read().unwrap();
        snapshot.parked.offer(full);
        pack_parked(&snapshot)
    }

    /// Moving cars and, when something changed, the parked chunks of the same snapshot, so no
    /// car shows twice or vanishes across a bay handoff. O(visible cars + unseen parked chunks);
    /// nothing for the parked side when no chunk changed. Never waits for the simulation.
    #[func]
    pub fn get_vehicle_frame(&self, full: bool) -> VarDictionary {
        let snapshot = self.snapshot.read().unwrap();
        let mut result = VarDictionary::new();
        result.set(
            "cars",
            crate::nodes::sim::bridge::agents::get_car_render_data(&snapshot),
        );
        if snapshot.parked.offer(full) {
            result.set("parked", pack_parked(&snapshot));
        }
        result
    }

    /// Records that the renderer installed a payload. O(1), lock-free; an answer from a replaced
    /// world or an older serial changes nothing, so it cannot discard a newer pose.
    #[func]
    pub fn acknowledge_parked_cars(&self, epoch: i64, serial: i64) {
        let snapshot = self.snapshot.read().unwrap();
        if snapshot.vehicle_world_epoch == epoch as u64 {
            snapshot.parked.acknowledge(serial as u64);
        }
    }
}

fn pack_parked(snapshot: &crate::nodes::sim::core::RenderSnapshot) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("epoch", snapshot.vehicle_world_epoch as i64);
    result.set("serial", snapshot.parked.serial as i64);
    result.set("full", snapshot.parked.holds_full());
    let mut updates = VarArray::new();
    for parked in snapshot.parked.unseen() {
        let (key, chunk) = (parked.key, &parked.chunk);
        let mut update = VarDictionary::new();
        update.set("chunk", Vector2i::new(key.0, key.1));
        update.set("version", chunk.version as i64);
        let mut buckets = VarDictionary::new();
        let mut identities = VarDictionary::new();
        for (bucket, poses, ids) in chunk.buckets() {
            buckets.set(bucket as i32, PackedFloat32Array::from(poses));
            identities.set(
                bucket as i32,
                PackedInt64Array::from_iter(ids.iter().map(|&id| id as i64)),
            );
        }
        update.set("buckets", buckets);
        update.set("identities", identities);
        updates.push(&update.to_variant());
    }
    result.set("chunks", updates);
    result
}
