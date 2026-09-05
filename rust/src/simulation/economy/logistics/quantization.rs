// SPDX-License-Identifier: GPL-2.0-only

//! Truckload quantization for aggregate freight amounts.

const MIN_POSITIVE_AMOUNT: f32 = 0.000_1;

pub(super) fn quantize_requested_amount(
    desired_amount: f32,
    available_amount: f32,
    affordable_amount: f32,
    min_shipment_units: f32,
    allow_emergency: bool,
    truck_load_units: f32,
) -> Option<f32> {
    let max_amount = desired_amount.min(available_amount).min(affordable_amount);
    quantize_capped_amount(
        max_amount,
        min_shipment_units,
        allow_emergency,
        truck_load_units,
    )
}

pub(super) fn quantize_export_amount(
    available_amount: f32,
    min_shipment_units: f32,
    truck_load_units: f32,
) -> Option<f32> {
    quantize_capped_amount(
        available_amount,
        min_shipment_units,
        false,
        truck_load_units,
    )
}

fn quantize_capped_amount(
    max_amount: f32,
    min_shipment_units: f32,
    allow_emergency: bool,
    truck_load_units: f32,
) -> Option<f32> {
    if max_amount <= MIN_POSITIVE_AMOUNT {
        return None;
    }
    let quantum = truck_load_units.max(MIN_POSITIVE_AMOUNT);
    let quantized = (max_amount / quantum).floor() * quantum;
    if quantized <= MIN_POSITIVE_AMOUNT {
        return None;
    }
    if quantized >= min_shipment_units || allow_emergency {
        Some(quantized)
    } else {
        None
    }
}
