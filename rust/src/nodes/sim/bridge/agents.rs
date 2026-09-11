// SPDX-License-Identifier: GPL-2.0-only

//! Godot-Rust bridge helpers for agent renderer data formatting.
use crate::nodes::sim::core::RenderSnapshot;
use godot::prelude::*;

/// Returns a Dictionary of packed transforms for visible agents, keyed by type.
pub fn get_agent_transforms(snapshot: &RenderSnapshot) -> VarDictionary {
    let mut dict = VarDictionary::new();
    for (&k, v) in &snapshot.pedestrian_transforms {
        dict.set(k as i32, PackedFloat32Array::from_iter(v.iter().cloned()));
    }
    dict
}

/// Exports car transforms, identities and support flags from a single coherent snapshot.
pub fn get_car_render_data(snapshot: &RenderSnapshot) -> VarDictionary {
    let mut dict = VarDictionary::new();
    for (&k, v) in &snapshot.car_transforms {
        let mut bucket = VarDictionary::new();
        bucket.set("transforms", PackedFloat32Array::from(v.as_slice()));
        bucket.set(
            "ids",
            PackedInt64Array::from(snapshot.car_render_ids[&k].as_slice()),
        );
        bucket.set(
            "ground_flags",
            PackedByteArray::from(snapshot.car_ground_flags[&k].as_slice()),
        );
        dict.set(k as i32, bucket);
    }
    dict
}
