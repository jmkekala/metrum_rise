// SPDX-License-Identifier: GPL-2.0-only

//! Engine-independent context-action rules and deterministic authoring names/angles.
//! Selection queries are O(selected objects); names are resolved once per command.

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ObjectKind {
    Mesh,
    Entrance,
    Driveway,
    Parking,
    Loading,
    Yard,
}

impl ObjectKind {
    pub(crate) fn parse(kind: &str, anchor: &str) -> Option<Self> {
        match (kind, anchor) {
            ("mesh", _) => Some(Self::Mesh),
            ("surface", _) => Some(Self::Yard),
            ("anchor", "entrance") => Some(Self::Entrance),
            ("anchor", "driveway") => Some(Self::Driveway),
            ("anchor", "parking") => Some(Self::Parking),
            ("anchor", "loading_bay") => Some(Self::Loading),
            _ => None,
        }
    }

    pub(crate) fn rotates(self) -> bool {
        matches!(
            self,
            Self::Mesh | Self::Entrance | Self::Parking | Self::Loading
        )
    }

    pub(crate) fn duplicates(self) -> bool {
        self != Self::Entrance
    }
}

pub(crate) fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    let base = if base.trim().is_empty() {
        "Object"
    } else {
        base.trim()
    };
    if used.insert(base.to_owned()) {
        return base.to_owned();
    }
    for suffix in 2u64.. {
        let candidate = format!("{base} {suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

pub(crate) fn rotation_degrees(angle: f32) -> f32 {
    // Explicit cardinal magnetism matches the existing authoring gesture, not LOD policy.
    let angle = (angle + 180.0).rem_euclid(360.0) - 180.0;
    let cardinal = (angle / 90.0).round() * 90.0;
    if (angle - cardinal).abs() <= 4.0 {
        cardinal
    } else {
        (angle * 10.0).round() / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrained_objects_never_advertise_unsupported_actions() {
        assert!(!ObjectKind::Driveway.rotates());
        assert!(!ObjectKind::Yard.rotates());
        assert!(!ObjectKind::Entrance.duplicates());
        assert!(ObjectKind::Entrance.rotates());
        assert!(ObjectKind::Mesh.duplicates());
        assert!(ObjectKind::parse("ghost", "").is_none());
    }

    #[test]
    fn names_and_angles_are_repeatable() {
        let mut used = HashSet::from(["Parking".into(), "Parking 2".into()]);
        assert_eq!(unique_name("Parking", &mut used), "Parking 3");
        assert_eq!(unique_name("Parking", &mut used), "Parking 4");
        assert_eq!(rotation_degrees(448.0), 90.0);
        assert_eq!(rotation_degrees(-448.0), -90.0);
        assert_eq!(rotation_degrees(45.0), 45.0);
    }
}
