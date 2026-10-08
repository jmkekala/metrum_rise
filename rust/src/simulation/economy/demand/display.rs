// SPDX-License-Identifier: GPL-2.0-only

//! Bounded HUD summaries of recorded decisions; never reruns economic or placement checks.

use std::fmt::Write;

use super::diagnostics::{BuildingActionDiagnostics, HouseholdAdmissionDiagnostics};
use super::system::DemandSystem;

impl DemandSystem {
    /// Three R/C/I project counts and explanations from the last hourly assessment.
    /// O(1) in city size. Strings allocate only when the UI requests presentation data.
    pub(crate) fn growth_display(&self) -> [(usize, String); 3] {
        self.last_building_action_diagnostics
            .iter()
            .map(|(use_kind, record)| {
                describe_growth(
                    record,
                    &self.last_admission_diagnostics,
                    use_kind as usize == 0,
                    self.cheat_max_demands_enabled,
                )
            })
    }
}

fn describe_growth(
    record: BuildingActionDiagnostics,
    admission: &HouseholdAdmissionDiagnostics,
    residential: bool,
    cheat: bool,
) -> (usize, String) {
    if !record.assessed {
        return (
            0,
            "Waiting for the next hourly growth assessment.".to_owned(),
        );
    }
    let projects = record.spawn_selected + record.queued as usize + record.constructing as usize;
    let mut text = format!(
        "Latest hourly assessment\n{} newly selected · {} queued · {} under construction",
        record.spawn_selected, record.queued, record.constructing,
    );
    if residential {
        let _ = write!(
            text,
            "\n{} household places needed before selecting homes.",
            record.housing_needed_slots
        );
        let _ = write!(
            text,
            "\n{} ready vacant homes · {} live funded jobs available to arrivals.",
            admission.vacant_household_slots, admission.move_in_job_slots
        );
        if admission.connected_border_count == 0 {
            text.push_str("\nArrivals need an outside road connection.");
        }
        if admission.income_rejections > 0 {
            let _ = write!(
                text,
                "\n{} applicants could not cover household costs from recurring income.",
                admission.income_rejections
            );
        }
        if admission.worker_housing_rejections > 0 {
            let _ = write!(
                text,
                "\n{} applicants deferred to preserve homes reserved for workers.",
                admission.worker_housing_rejections
            );
        }
        if record.spawn_not_needed > 0 {
            let _ = write!(
                text,
                "\n{} home sites unused: current needs and workplace reservations are covered.",
                record.spawn_not_needed
            );
        }
    }
    if !record.sites.has_assets {
        text.push_str("\nNo eligible building assets for this use.");
    }
    if record.sites.free_lots == 0 {
        text.push_str("\nNo free zoned lots for this use.");
    } else {
        let _ = write!(
            text,
            "\n{} free zoned lots · {} unqueued buildable candidates.",
            record.sites.free_lots, record.spawn_candidates
        );
    }
    for (count, reason) in [
        (
            record.sites.missing_asset_lots,
            "lots have no eligible asset for their zoning profile",
        ),
        (
            record.sites.unusable_lots,
            "lots cannot fit a legal building with usable road access and terrain",
        ),
        (
            record.spawn_rejected_absorption,
            "preferred proposals lack unreserved customers",
        ),
        (
            record.spawn_rejected_viability,
            "preferred proposals lack enough operating income",
        ),
        (
            record.spawn_rejected_workforce,
            "preferred proposals cannot supply their workforce by completion",
        ),
    ] {
        if count > 0 {
            let _ = write!(text, "\n{count} {reason}.");
        }
    }
    if record.spawn_rejected_absorption
        + record.spawn_rejected_viability
        + record.spawn_rejected_workforce
        > 0
    {
        text.push_str("\nNo viable alternative was found on the rejected sites.");
    }
    if cheat {
        text.push_str("\nGrowth cheat enabled: economic entry checks are bypassed.");
    }
    (projects, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::demand::SpawnSiteAvailability;

    #[test]
    fn display_reports_missing_land_assets_and_economic_constraints_without_pressure() {
        let admission = HouseholdAdmissionDiagnostics::default();
        let mut record = BuildingActionDiagnostics {
            assessed: true,
            ..Default::default()
        };
        let (projects, text) = describe_growth(record, &admission, false, false);
        assert_eq!(projects, 0);
        assert!(text.contains("No free zoned lots"));
        assert!(text.contains("No eligible building assets"));
        record.sites = SpawnSiteAvailability {
            has_assets: true,
            free_lots: 9,
            missing_asset_lots: 2,
            unusable_lots: 1,
        };
        record.spawn_candidates = 6;
        record.spawn_selected = 1;
        record.queued = 2;
        record.constructing = 3;
        record.spawn_rejected_absorption = 2;
        record.spawn_rejected_viability = 1;
        record.spawn_rejected_workforce = 2;
        let (projects, text) = describe_growth(record, &admission, false, false);
        assert_eq!(projects, 6);
        for reason in [
            "1 newly selected",
            "2 queued",
            "3 under construction",
            "2 lots have no eligible asset",
            "1 lots cannot fit",
            "2 preferred proposals lack unreserved customers",
            "1 preferred proposals lack enough operating income",
            "2 preferred proposals cannot supply",
        ] {
            assert!(text.contains(reason), "{text}");
        }
        assert!(!text.contains('%'));
    }

    #[test]
    fn waiting_and_housing_constraints_are_explicit() {
        let record = BuildingActionDiagnostics::default();
        let admission = HouseholdAdmissionDiagnostics {
            income_rejections: 2,
            worker_housing_rejections: 1,
            ..Default::default()
        };
        assert!(
            describe_growth(record, &admission, true, false)
                .1
                .contains("Waiting")
        );
        let record = BuildingActionDiagnostics {
            assessed: true,
            housing_needed_slots: 7,
            ..record
        };
        let (_, text) = describe_growth(record, &admission, true, true);
        for reason in [
            "7 household places needed",
            "outside road connection",
            "2 applicants could not cover",
            "1 applicants deferred",
            "Growth cheat enabled",
        ] {
            assert!(text.contains(reason), "{text}");
        }
    }
}
