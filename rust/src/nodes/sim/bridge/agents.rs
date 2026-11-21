// SPDX-License-Identifier: GPL-3.0-or-later

//! Godot-Rust bridge helpers for agent renderer data formatting.
use crate::nodes::sim::core::RenderSnapshot;
use godot::prelude::*;

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

/// The catalogue as `SimulationNode.get_vehicle_catalogue` hands it to GDScript.
pub(crate) fn vehicle_catalogue(
    catalogue: &crate::nodes::sim::render::vehicle_catalogue::VehicleCatalogue,
) -> VarDictionary {
    use crate::assets::LampGroup;
    use crate::nodes::sim::render::vehicle_catalogue::turned;
    let mut looks = VarArray::new();
    for look in &catalogue.looks {
        let model = &catalogue.models[usize::from(look.model)];
        let data = &model.data;
        let variant = &data.variants[usize::from(look.variant)];
        let mut entry = VarDictionary::new();
        entry.set("id", model.id.as_str());
        entry.set("model", i64::from(look.model));
        entry.set(
            "tiers",
            model.tiers.iter().map(|tier| GString::from(model.file(tier).as_str())).collect::<PackedStringArray>(),
        );
        entry.set("tier_starts_m", PackedFloat32Array::from(model.tier_starts_m.as_slice()));
        entry.set("albedo", model.file(&variant.albedo).as_str());
        entry.set("orm", model.file(&data.orm).as_str());
        entry.set("normal", data.normal.as_deref().map(|n| model.file(n)).unwrap_or_default().as_str());
        entry.set("emission", model.file(&data.emission).as_str());
        entry.set("paint", variant.paint);
        entry.set(
            "palette",
            model.palette.iter().map(|&[r, g, b]| Color::from_rgb(r, g, b)).collect::<PackedColorArray>(),
        );
        entry.set(
            "wheels",
            data.wheels
                .iter()
                .map(|wheel| {
                    let [x, y, z] = turned(wheel.centre);
                    Vector4::new(x, y, z, if wheel.steer { 1.0 } else { 0.0 })
                })
                .collect::<PackedVector4Array>(),
        );
        let head = model.lamp(&[LampGroup::LowBeam], 1.0);
        let tail = model.lamp(&[LampGroup::TailBrake, LampGroup::Tail], -1.0);
        let [length, width, height] = model.size;
        entry.set("head_lamp", head.unwrap_or(Vector3::new(width * 0.35, height * 0.45, -length * 0.5)));
        entry.set("tail_lamp", tail.unwrap_or(Vector3::new(width * 0.35, height * 0.45, length * 0.5)));
        entry.set("size", Vector3::new(length, width, height));
        looks.push(&entry.to_variant());
    }
    let mut result = VarDictionary::new();
    result.set("revision", catalogue.revision as i64);
    result.set("looks", looks);
    result
}
