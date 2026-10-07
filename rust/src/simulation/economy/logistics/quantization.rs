// SPDX-License-Identifier: GPL-2.0-only

//! Resource-specific batch rounding and the physical payload limit for a single freight vehicle.

const MIN_POSITIVE_AMOUNT: f32 = 0.000_1;

/// Accumulate one production day's net output, rounded up to a legal batch and
/// capped at the largest legal load. A small producer need not fill an entire truck.
pub(super) fn export_dispatch_target_kg(
    daily_net_output: f32,
    shipment_batch_kg: f32,
    truck_payload_kg: f32,
) -> f32 {
    let full_load = (truck_payload_kg / shipment_batch_kg).floor() * shipment_batch_kg;
    ((daily_net_output / shipment_batch_kg).ceil().max(1.0) * shipment_batch_kg).min(full_load)
}

pub(super) fn quantize_requested_amount(
    desired_amount: f32,
    available_amount: f32,
    affordable_amount: f32,
    shipment_batch_kg: f32,
    allow_emergency: bool,
    truck_payload_kg: f32,
) -> Option<f32> {
    let max_amount = desired_amount.min(available_amount).min(affordable_amount);
    quantize_capped_amount(
        max_amount,
        shipment_batch_kg,
        allow_emergency,
        truck_payload_kg,
    )
}

pub(super) fn quantize_export_amount(
    available_amount: f32,
    shipment_batch_kg: f32,
    truck_payload_kg: f32,
) -> Option<f32> {
    quantize_capped_amount(available_amount, shipment_batch_kg, false, truck_payload_kg)
}

/// Whether unreserved stock can fill a legal batch within one physical vehicle's payload.
pub(super) fn fills_one_batch(amount: f32, batch_kg: f32, truck_payload_kg: f32) -> bool {
    quantize_capped_amount(amount, batch_kg, true, truck_payload_kg).is_some()
}

fn quantize_capped_amount(
    max_amount: f32,
    shipment_batch_kg: f32,
    allow_emergency: bool,
    truck_payload_kg: f32,
) -> Option<f32> {
    if max_amount <= MIN_POSITIVE_AMOUNT || shipment_batch_kg <= 0.0 {
        return None;
    }
    // One shipment owns exactly one carrier. Excess stock remains for later orders;
    // never allocate a list of trucks or duplicate the full payment for a split load.
    let quantum = shipment_batch_kg;
    let max_amount = max_amount.min(truck_payload_kg);
    let quantized = ((max_amount / quantum).floor() * quantum).min(max_amount);
    if quantized <= MIN_POSITIVE_AMOUNT {
        return None;
    }
    if quantized >= shipment_batch_kg || allow_emergency {
        Some(quantized)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_dispatch_lots_follow_daily_output_within_payload() {
        assert_eq!(export_dispatch_target_kg(200.0, 50.0, 20_000.0), 200.0);
        assert_eq!(export_dispatch_target_kg(120.0, 40.0, 20_000.0), 120.0);
        assert_eq!(export_dispatch_target_kg(12.0, 50.0, 20_000.0), 50.0);
        assert_eq!(
            export_dispatch_target_kg(36_000.0, 50.0, 20_000.0),
            20_000.0
        );
        assert_eq!(
            export_dispatch_target_kg(36_000.0, 300.0, 20_000.0),
            19_800.0
        );
    }

    #[test]
    fn batch_size_and_vehicle_payload_are_independent() {
        assert_eq!(
            quantize_export_amount(36_250.0, 50.0, 20_000.0),
            Some(20_000.0)
        );
        assert_eq!(
            quantize_export_amount(16_250.0, 50.0, 20_000.0),
            Some(16_250.0)
        );
        assert_eq!(quantize_export_amount(400.0, 50.0, 20_000.0), Some(400.0));
        assert_eq!(quantize_export_amount(49.0, 50.0, 20_000.0), None);
        assert_eq!(
            quantize_requested_amount(30_000.0, 28_000.0, 25_000.0, 100.0, false, 20_000.0),
            Some(20_000.0)
        );
        assert_eq!(
            quantize_requested_amount(30_000.0, 28_000.0, 340.0, 100.0, false, 20_000.0),
            Some(300.0)
        );
        assert_eq!(
            quantize_export_amount(40.0, 0.0, 20_000.0),
            None,
            "services cannot become cargo"
        );
    }
}
