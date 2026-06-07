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
