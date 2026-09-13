// SPDX-License-Identifier: GPL-2.0-only

//! Shared helpers for authored economy validation.

use std::collections::BTreeSet;

pub(super) fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.to_owned()) {
            duplicates.insert(id.to_owned());
        }
    }
    duplicates.into_iter().collect()
}

/// Rejects non-finite values and values outside the authored inclusive range.
pub(in crate::simulation::economy::definitions) fn validate_range(
    value: f32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}
