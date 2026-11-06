// SPDX-License-Identifier: GPL-2.0-only

//! Thin parking inspector, debug fleet and versioned parked-render bridge.

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
        let count = core.parking.fill_test_fleet();
        core.build_parked_chunks();
        count as i32
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

    /// Nonblocking O(pending dirty payload); unchanged chunks send nothing. Full reconnects
    /// explicitly request all chunks. Payloads survive skipped frames until exact-version ack.
    #[func]
    pub fn get_parked_car_chunks(&self, full: bool) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("busy", true);
        let Some(mut core) = self.try_lock_core() else {
            return result;
        };
        core.build_parked_chunks();
        let epoch = core.world_epoch;
        let chunks = core.parking.publications(full);
        drop(core);
        pack_parked(epoch, chunks)
    }

    /// Returns moving and parked payloads from the same custody publication. If a newer
    /// parking transition has overtaken the moving snapshot, both renderers retain their
    /// previous frame. O(visible cars + dirty parked chunks), nonblocking on the simulation.
    #[func]
    pub fn get_vehicle_frame(&self, full: bool) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("busy", true);
        let snapshot = self.snapshot.read().unwrap();
        let Some(mut core) = self.try_lock_core() else {
            return result;
        };
        if snapshot.parking_epoch != core.parking.publication_epoch()
            || snapshot.vehicle_world_epoch != core.world_epoch
        {
            return result;
        }
        core.build_parked_chunks();
        let chunks = core.parking.publications(full);
        let epoch = core.world_epoch;
        drop(core);
        result.set(
            "cars",
            crate::nodes::sim::bridge::agents::get_car_render_data(&snapshot),
        );
        result.set("parked", pack_parked(epoch, chunks));
        result.set("busy", false);
        result
    }

    /// O(log pending chunks); stale world/version acknowledgements cannot discard a newer pose.
    #[func]
    pub fn acknowledge_parked_car_chunk(&mut self, epoch: i64, key: Vector2i, version: i64) {
        let Some(mut core) = self.try_lock_core() else {
            return;
        };
        if core.world_epoch == epoch as u64 {
            core.parking.acknowledge((key.x, key.y), version as u64);
        }
    }
}

fn pack_parked(
    epoch: u64,
    chunks: Vec<(
        (i32, i32),
        std::sync::Arc<crate::simulation::parking::ParkedChunk>,
    )>,
) -> VarDictionary {
    let mut result = VarDictionary::new();
    result.set("busy", false);
    result.set("epoch", epoch as i64);
    let mut updates = VarArray::new();
    for (key, chunk) in chunks {
        let mut update = VarDictionary::new();
        update.set("chunk", Vector2i::new(key.0, key.1));
        update.set("version", chunk.version as i64);
        let mut buckets = VarDictionary::new();
        let mut identities = VarDictionary::new();
        for (&bucket, buffer) in &chunk.buckets {
            buckets.set(bucket as i32, PackedFloat32Array::from(buffer.as_slice()));
        }
        for (&bucket, ids) in &chunk.identities {
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
