// SPDX-License-Identifier: GPL-2.0-only

//! Tests for shipped economy runtime tuning validation.

#[test]
fn runtime_tuning_enforces_day_duration_without_narrowing() {
    use super::validation::validate_runtime_tuning;
    let mut tuning = (*super::load_runtime_economy_tuning().unwrap()).clone();
    for (duration, valid) in [
        (60.0, true),
        (1_440.0, true),
        (2_880.0, true),
        (60.0 - 1.0e-9, false),
        (0.0, false),
        (-1.0, false),
        (f64::NAN, false),
        (f64::INFINITY, false),
    ] {
        tuning.operational_clock.seconds_per_day = duration;
        assert_eq!(
            validate_runtime_tuning(&tuning).is_ok(),
            valid,
            "day duration {duration}"
        );
    }
}

#[test]
fn runtime_tuning_rejects_nonfinite_treasury_and_export_settings() {
    use super::validation::validate_runtime_tuning;
    let original = super::load_runtime_economy_tuning().unwrap();
    for field in 0..2 {
        for value in [-1.0, 0.0, f32::NAN, f32::INFINITY] {
            let mut tuning = (*original).clone();
            if field == 0 {
                tuning.logistics.owa_export_saturation_loads_to_floor = value;
            } else {
                tuning.logistics.owa_export_saturation_recovery_hours = value;
            }
            assert!(
                validate_runtime_tuning(&tuning).is_err(),
                "field {field} accepted {value}"
            );
        }
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut tuning = (*original).clone();
        tuning.startup_treasury_balance = value;
        assert!(
            validate_runtime_tuning(&tuning).is_err(),
            "treasury accepted {value}"
        );
    }
}
