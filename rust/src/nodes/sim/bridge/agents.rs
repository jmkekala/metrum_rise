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

/// Returns a Dictionary of packed transforms for visible car agents, keyed by type.
pub fn get_car_transforms(snapshot: &RenderSnapshot) -> VarDictionary {
    let mut dict = VarDictionary::new();
    for (&k, v) in &snapshot.car_transforms {
        dict.set(k as i32, PackedFloat32Array::from_iter(v.iter().cloned()));
    }
    dict
}

/// Returns render IDs for visible car agents, keyed to match `get_car_transforms`.
pub fn get_car_render_ids(snapshot: &RenderSnapshot) -> VarDictionary {
    let mut dict = VarDictionary::new();
    for (&k, v) in &snapshot.car_render_ids {
        dict.set(k as i32, PackedInt64Array::from_iter(v.iter().copied()));
    }
    dict
}
